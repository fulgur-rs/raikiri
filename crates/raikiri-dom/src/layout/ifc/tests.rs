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

#[test]
fn a_huge_font_size_or_line_height_lays_out_in_bounded_time() {
    // Untrusted CSS can ask for sizes that overflow to infinity on the way
    // in; the layout must still finish, without an error.
    for css in [
        "font-size:1e40px",
        "font-size:1e9px;line-height:1e30",
        "font-size:1e20px;letter-spacing:1e30px",
    ] {
        let css = css.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut doc, cascade, _) = ahem_paragraph("aaaa bbbb", &css);
            let laid_out =
                layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).is_ok();
            let _ = tx.send(laid_out);
        });
        let laid_out = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the layout finishes");
        assert!(laid_out);
    }
}

/// The block-direction offset of a 2px square `tag` element that is the only
/// child of a zero-margin body (plus `text` after it), in a document of the
/// given quirks mode.
fn replaced_offset_y(mode: raikiri_traits::QuirksMode, tag: &str, text: &str) -> f32 {
    use taffy::Style;
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    doc.append_text(body, " ");
    let replaced = doc.append_element(
        Some(body),
        tag,
        Style::default(),
        Some("width:2px;height:2px"),
    );
    doc.append_text(body, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    doc.nodes[replaced].unrounded_layout.location.y
}

#[test]
fn quirks_mode_line_of_only_replaced_elements_has_no_strut() {
    use raikiri_traits::QuirksMode;
    // Quirks Mode Standard 3.3: a paragraph holding no text other than
    // collapsed white space does not make room for the root inline box's
    // strut, so a short replaced element sits at the top of its line.
    for mode in [QuirksMode::Quirks, QuirksMode::LimitedQuirks] {
        for tag in ["video", "img", "canvas"] {
            assert_eq!(replaced_offset_y(mode, tag, " "), 0.0, "{tag} in {mode:?}");
        }
    }
    // In no-quirks mode the strut keeps the element on the baseline, below
    // the top of the line.
    assert!(replaced_offset_y(QuirksMode::NoQuirks, "video", " ") > 1.0);
    // Text in the paragraph keeps the strut in quirks mode as well.
    assert!(replaced_offset_y(QuirksMode::Quirks, "video", "x") > 1.0);
}
