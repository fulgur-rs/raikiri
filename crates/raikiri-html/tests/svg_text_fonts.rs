//! SVG text is set in the faces of the font collection that HTML text uses.

use raikiri_html::{FontCollectionBuilder, RenderFonts};
use raikiri_svg::{
    SvgDocument, SvgFontFamily, SvgFontRequest, SvgFontSource, SvgFontStyle, SvgRootStyle,
    SvgViewport,
};
use raikiri_traits::vector::{Group, Node, TextRun, VectorImage};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");
/// Has a space and two Thai digits, and none of the letters.
const SPARSE: &[u8] = include_bytes!("data/NotoSansTest-Regular.ttf");

const VIEWPORT: SvgViewport = SvgViewport {
    width: 40.0,
    height: 20.0,
};

/// The sparse font first, so it is first among the generic families too.
fn fonts() -> RenderFonts {
    FontCollectionBuilder::new()
        .font_bytes("Sparse", SPARSE)
        .font_bytes("Ahem", AHEM)
        .build()
        .expect("both fonts are valid")
}

fn scene(body: &str, fonts: &RenderFonts) -> VectorImage {
    let source =
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"##);
    SvgDocument::parse(source.as_bytes())
        .unwrap()
        .scene_with_fonts(VIEWPORT, SvgRootStyle::default(), None, fonts)
        .unwrap()
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

fn named(name: &str) -> SvgFontRequest {
    SvgFontRequest::new(vec![SvgFontFamily::Named(name.into())])
}

#[test]
fn text_uses_the_named_family_of_the_collection() {
    let fonts = fonts();
    let scene = scene(
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000">AB</text>"##,
        &fonts,
    );
    let runs = runs(&scene.root);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].glyphs.len(), 2);
    for glyph in &runs[0].glyphs {
        assert_eq!(glyph.font.bytes(), AHEM);
    }
}

#[test]
fn generic_families_map_onto_the_bundle_like_they_do_for_html() {
    let fonts = fonts();
    // The sparse font is first in the bundle, and so is first among the
    // generic families; it has no `A`, so the text falls to the next face.
    for family in [
        "serif",
        "sans-serif",
        "monospace",
        "cursive",
        "fantasy",
        "No Such Family",
    ] {
        let scene = scene(
            &format!(
                r##"<text x="2" y="12" font-family="{family}" font-size="8" fill="#000">AB</text>"##
            ),
            &fonts,
        );
        let runs = runs(&scene.root);
        assert_eq!(runs.len(), 1, "{family}");
        assert!(
            runs[0]
                .glyphs
                .iter()
                .all(|glyph| glyph.font.bytes() == AHEM),
            "{family}"
        );
    }
}

#[test]
fn a_face_is_one_allocation_however_often_it_is_asked_for() {
    let fonts = fonts();
    let first = fonts.select(&named("Ahem")).unwrap();
    let second = fonts.select(&named("Ahem")).unwrap();
    assert!(first.is_same_face(&second));
    // Clones of the set share the faces handed out.
    assert!(
        fonts
            .clone()
            .select(&named("Ahem"))
            .unwrap()
            .is_same_face(&first)
    );
    // And scenes built on different calls share them too.
    let body = r##"<text x="2" y="12" font-family="Ahem" font-size="8" fill="#000">A</text>"##;
    let (one, two) = (scene(body, &fonts), scene(body, &fonts));
    assert!(
        runs(&one.root)[0].glyphs[0]
            .font
            .is_same_face(&runs(&two.root)[0].glyphs[0].font)
    );
    assert!(runs(&one.root)[0].glyphs[0].font.is_same_face(&first));
    assert!(!first.is_same_face(&fonts.select(&named("Sparse")).unwrap()));
}

#[test]
fn a_character_the_face_lacks_comes_from_the_next_face_of_the_bundle() {
    let fonts = fonts();
    let sparse = fonts.select(&named("Sparse")).unwrap();
    assert_eq!(sparse.bytes(), SPARSE);
    let mut request = named("Sparse");
    request.families.push(SvgFontFamily::Serif);
    let fallback = fonts.fallback(&request, 'A').unwrap();
    assert_eq!(fallback.bytes(), AHEM);
    // A character no face has gets no fallback.
    assert!(fonts.fallback(&request, '\u{1f600}').is_none());
}

#[test]
fn the_requested_weight_width_and_slant_reach_the_collection() {
    let fonts = fonts();
    let mut request = named("Ahem");
    request.weight = 700;
    request.stretch = 75.0;
    // Only one face is bundled, so any slant still gets it.
    for style in [
        SvgFontStyle::Normal,
        SvgFontStyle::Italic,
        SvgFontStyle::Oblique,
    ] {
        request.style = style;
        assert_eq!(fonts.select(&request).unwrap().bytes(), AHEM);
    }
}

#[test]
fn a_character_of_another_script_is_asked_for_by_that_script() {
    let fonts = fonts();
    let mut request = named("Ahem");
    request.families.push(SvgFontFamily::Serif);
    // Thai digits: the sparse font has them and Ahem does not.
    let thai = fonts.fallback(&request, '\u{e70}').unwrap();
    assert_eq!(thai.bytes(), SPARSE);
}
