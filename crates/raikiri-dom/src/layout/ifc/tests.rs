#[test]
fn shodo_defaults_that_the_ifc_path_relies_on_are_stable() {
    // The exact version pin in the workspace manifest is the real contract;
    // this fails loudly when a bump changes the bounded defaults.
    let limits = shodo::limits::Limits::default();
    assert_eq!(limits.max_warnings, Some(1024));
    assert!(limits.max_text_bytes.is_some());
}

use crate::Document;
use crate::fonts::{FontFaceLoader, build_inline_document_fonts};
use crate::layout::layout_single_page;
use crate::layout::test_support::with_ahem;
use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};
use raikiri_style::FontFaceRegistry;

const CANVAS_TEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/CanvasTest-nospace.ttf"
));

/// The layer of the font each glyph run of the root's lines used.
fn run_layers(doc: &Document, root: usize) -> Vec<u32> {
    doc.nodes[root]
        .ifc_lines()
        .expect("lines")
        .iter()
        .flat_map(|line| line.fragments())
        .filter_map(|fragment| match fragment {
            shodo::Fragment::GlyphRun(run) => Some(run.font().layer()),
            _ => None,
        })
        .collect()
}

fn layer_of_ahem(shared: &shodo::font::FontCollection) -> u32 {
    shared
        .match_cluster(
            &shodo::font::FontQuery {
                families: vec![shodo::style::FontFamily::Named("Ahem".to_owned())],
                ..Default::default()
            },
            "A",
        )
        .expect("the shared layer holds Ahem")
        .id
        .layer()
}

/// Serves the canvas test font for every `url()`.
struct CanvasLoader;

impl FontFaceLoader for CanvasLoader {
    fn load(&self, url: &str) -> Option<Vec<u8>> {
        (url == "face.ttf").then(|| CANVAS_TEST.to_vec())
    }
}

#[test]
fn a_document_layer_face_supplies_the_glyphs_of_the_paragraph() {
    let registry =
        FontFaceRegistry::from_source("@font-face { font-family: Face; src: url(face.ttf); }");
    let shared = ifc_ahem_fonts();
    let ahem_layer = layer_of_ahem(&shared);
    let (layer, report) = build_inline_document_fonts(&shared, &registry, &CanvasLoader);
    assert_eq!(report.applied, vec!["Face".to_owned()]);

    let (mut doc, cascade, root) =
        ahem_paragraph("ABC", "font-family:Face;font-size:20px;line-height:20px");
    doc.set_font_collection_with_limits(layer, shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let layers = run_layers(&doc, root);
    assert!(!layers.is_empty());
    // Every run came from the document layer, none from the shared Ahem.
    assert!(
        layers.iter().all(|&l| l != ahem_layer),
        "{layers:?} vs {ahem_layer}"
    );
}

#[test]
fn a_family_the_document_does_not_declare_still_resolves_in_the_shared_layer() {
    let registry =
        FontFaceRegistry::from_source("@font-face { font-family: Face; src: url(face.ttf); }");
    let shared = ifc_ahem_fonts();
    let ahem_layer = layer_of_ahem(&shared);
    let (layer, report) = build_inline_document_fonts(&shared, &registry, &CanvasLoader);
    assert_eq!(report.applied, vec!["Face".to_owned()]);
    // `Ahem` is not declared by the document, so the layer hands it to the
    // shared layer it sits on.
    let (mut doc, cascade, root) = ahem_paragraph("ABC", "font-size:20px;line-height:20px");
    doc.set_font_collection_with_limits(layer, shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let layers = run_layers(&doc, root);
    assert!(!layers.is_empty());
    assert!(layers.iter().all(|&l| l == ahem_layer), "{layers:?}");
}

#[test]
fn a_document_layer_face_is_invisible_to_the_shared_layer() {
    let registry =
        FontFaceRegistry::from_source("@font-face { font-family: Face; src: url(face.ttf); }");
    let shared = ifc_ahem_fonts();
    let generation = shared.generation();
    let (_layer, report) = build_inline_document_fonts(&shared, &registry, &CanvasLoader);
    assert_eq!(report.applied, vec!["Face".to_owned()]);
    assert_eq!(shared.generation(), generation);
    // Another document built on the same shared layer does not see `Face`:
    // its paragraph falls back to the shared Ahem.
    let ahem_layer = layer_of_ahem(&shared);
    let (other, _) =
        build_inline_document_fonts(&shared, &FontFaceRegistry::from_source(""), &CanvasLoader);
    let (mut doc, cascade, root) = ahem_paragraph(
        "ABC",
        "font-family:Face, Ahem;font-size:20px;line-height:20px",
    );
    doc.set_font_collection_with_limits(other, shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let layers = run_layers(&doc, root);
    assert!(!layers.is_empty());
    assert!(layers.iter().all(|&l| l == ahem_layer), "{layers:?}");
}

#[test]
fn a_document_without_explicit_fonts_uses_the_engine() {
    // No fonts are given to the document: the first layout switches the
    // engine on with the installed fonts.
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.has_font_collection());
}

#[test]
fn a_paragraph_the_engine_cannot_project_is_an_error() {
    // Four text bytes are allowed per paragraph; "aaaa bbbb" has nine.
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "");
    doc.set_font_collection_with_limits(
        ifc_ahem_fonts(),
        shodo::limits::Limits {
            max_text_bytes: Some(4),
            ..shodo::limits::Limits::default()
        },
    );
    let result = layout_single_page(&mut doc, &cascade, page_box_800x600());
    assert!(
        matches!(result, Err(raikiri_traits::LayoutError::IfcLimitExceeded { node, .. }) if node == root),
        "{result:?}"
    );
}
