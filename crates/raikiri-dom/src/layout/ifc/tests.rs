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

fn image_line_geometry(
    mode: raikiri_traits::QuirksMode,
    wrapper_css: Option<&str>,
    vertical_align: &str,
    second_line_text: bool,
) -> (f32, Vec<f32>) {
    use taffy::Style;
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let parent = if let Some(css) = wrapper_css {
        doc.append_element(Some(body), "span", Style::default(), Some(css))
    } else {
        body
    };
    let image = doc.append_element(
        Some(parent),
        "img",
        Style::default(),
        Some(&format!(
            "width:2px;height:2px;vertical-align:{vertical_align}"
        )),
    );
    if second_line_text {
        doc.append_element(Some(body), "br", Style::default(), None::<&str>);
        doc.append_text(body, "x");
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let heights = doc.nodes[body]
        .ifc_lines()
        .expect("body lines")
        .iter()
        .map(shodo::Line::block_size)
        .collect();
    (doc.nodes[image].unrounded_layout.location.y, heights)
}

fn mixed_image_line_geometry(
    mode: raikiri_traits::QuirksMode,
    wrapped: bool,
    vertical_aligns: &[&str],
) -> (Vec<f32>, Vec<(f32, f32)>) {
    use taffy::Style;
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let mut images = Vec::new();
    for (line, vertical_align) in vertical_aligns.iter().enumerate() {
        if matches!(line, 1 | 3) {
            doc.append_text(body, "x");
        }
        let parent = if wrapped {
            doc.append_element(Some(body), "span", Style::default(), None::<&str>)
        } else {
            body
        };
        let image = doc.append_element(
            Some(parent),
            "img",
            Style::default(),
            Some(&format!(
                "width:2px;height:2px;vertical-align:{}",
                vertical_align
            )),
        );
        images.push(image);
        if line == 3 {
            doc.append_text(body, "x");
        }
        if line != 4 {
            doc.append_element(Some(body), "br", Style::default(), None::<&str>);
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let positions = images
        .into_iter()
        .map(|image| doc.nodes[image].unrounded_layout.location.y)
        .collect();
    let lines = doc.nodes[body]
        .ifc_lines()
        .expect("body lines")
        .iter()
        .map(|line| (line.block_offset(), line.block_size()))
        .collect();
    (positions, lines)
}

#[test]
fn quirks_mode_text_free_inline_wrapper_has_no_strut() {
    use raikiri_traits::QuirksMode;
    for mode in [QuirksMode::Quirks, QuirksMode::LimitedQuirks] {
        let direct = image_line_geometry(mode, None, "baseline", false);
        let wrapped = image_line_geometry(mode, Some(""), "baseline", false);
        assert_eq!(wrapped, direct, "{mode:?}");
    }
    let standards = image_line_geometry(QuirksMode::NoQuirks, Some(""), "baseline", false);
    assert!(standards.1[0] > 2.0, "{standards:?}");
}

#[test]
fn quirks_mode_vertical_align_uses_zero_root_metrics() {
    use raikiri_traits::QuirksMode;
    for align in ["middle", "text-top", "text-bottom", "sub", "super"] {
        let direct = image_line_geometry(QuirksMode::Quirks, None, align, false);
        let wrapped = image_line_geometry(QuirksMode::Quirks, Some(""), align, false);
        assert_eq!(wrapped, direct, "{align}");
        let standards = image_line_geometry(QuirksMode::NoQuirks, None, align, false);
        assert!(
            standards.1[0] >= direct.1[0],
            "{align}: {standards:?} vs {direct:?}"
        );
    }
}

#[test]
fn quirks_mode_mixed_lines_keep_wrapped_images_in_their_direct_positions() {
    use raikiri_traits::QuirksMode;
    let mut mismatches = Vec::new();
    let alignments = ["middle", "text-top", "text-bottom", "sub", "super"];
    for mode in [QuirksMode::Quirks, QuirksMode::NoQuirks] {
        let direct = mixed_image_line_geometry(mode, false, &alignments);
        let wrapped = mixed_image_line_geometry(mode, true, &alignments);
        assert_eq!(direct.0.len(), 5, "{mode:?}: {direct:?}");
        assert_eq!(direct.1.len(), 5, "{mode:?}: {direct:?}");
        if wrapped != direct {
            mismatches.push((mode, direct, wrapped));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:?}");
}

#[test]
fn quirks_mode_a_forced_break_after_an_image_adds_no_strut() {
    use raikiri_traits::QuirksMode;
    // `<img><br>x`: in quirks mode the image line is as tall as the image, as
    // without the break, and only the text line gets the root's strut
    // (Chromium: the image at 0 and the text line at 2px).
    let empty = image_line_geometry(QuirksMode::Quirks, None, "baseline", false);
    let mixed = image_line_geometry(QuirksMode::Quirks, None, "baseline", true);
    assert_eq!(mixed.1, [2.0, 20.0], "{mixed:?}");
    assert_eq!(mixed.1[0], empty.1[0]);
    assert_eq!(mixed.0, empty.0);
    // In standards mode the root's strut holds the image line open.
    let standards = image_line_geometry(QuirksMode::NoQuirks, None, "baseline", true);
    assert_eq!(standards.1[0], 20.0, "{standards:?}");
}

#[test]
fn quirks_mode_inline_axis_padding_preserves_its_line_height() {
    use raikiri_traits::QuirksMode;
    let empty = image_line_geometry(QuirksMode::Quirks, Some(""), "baseline", false);
    let inline_padded = image_line_geometry(
        QuirksMode::Quirks,
        Some("padding-left:1px"),
        "baseline",
        false,
    );
    assert!(
        inline_padded.1[0] > empty.1[0],
        "{inline_padded:?} vs {empty:?}"
    );
    let block_padded = image_line_geometry(
        QuirksMode::Quirks,
        Some("padding-top:1px"),
        "baseline",
        false,
    );
    assert_eq!(block_padded.1[0], empty.1[0]);
}

fn nested_inline_line_height(mode: raikiri_traits::QuirksMode, inner_css: &str) -> f32 {
    use taffy::Style;
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let outer = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("line-height:40px"),
    );
    let inner = doc.append_element(Some(outer), "b", Style::default(), Some(inner_css));
    doc.append_text(inner, "x");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let line = &doc.nodes[body].ifc_lines().expect("lines")[0];
    line.block_size()
}

#[test]
fn quirks_mode_parent_inline_without_direct_text_has_no_strut() {
    use raikiri_traits::QuirksMode;
    // Neither the root nor the outer span directly contains text on the
    // line, so only the inner box's 10px strut counts (Quirks Mode Standard,
    // 3.3-3.4: the block's own line-height is ignored).
    assert_eq!(
        nested_inline_line_height(QuirksMode::Quirks, "line-height:10px"),
        10.0
    );
    assert_eq!(
        nested_inline_line_height(QuirksMode::NoQuirks, "line-height:10px"),
        40.0
    );
    assert_eq!(
        nested_inline_line_height(QuirksMode::Quirks, "display:contents;line-height:10px"),
        40.0
    );
}

#[test]
fn quirks_mode_generated_text_keeps_only_its_own_inline_strut() {
    use taffy::Style;
    let mut doc = Document::new();
    doc.set_quirks_mode(raikiri_traits::QuirksMode::Quirks);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, "span::before { content: \"x\"; line-height: 10px; }");
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("line-height:40px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let line = &doc.nodes[body].ifc_lines().expect("lines")[0];
    // Only the generated text's own box directly contains text, so its 10px
    // strut alone sets the line height; the root and the span add none.
    assert_eq!(line.block_size(), 10.0);
}

#[test]
fn a_line_break_is_placed_on_the_line_it_ends() {
    use taffy::Style;
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    doc.append_text(body, "aa");
    let first = doc.append_element(Some(body), "br", Style::default(), None::<&str>);
    doc.append_text(body, "bbbb");
    let span = doc.append_element(
        Some(body),
        "span",
        Style::default(),
        Some("line-height:40px"),
    );
    let second = doc.append_element(Some(span), "br", Style::default(), None::<&str>);
    doc.append_text(body, "c");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let heights: Vec<f32> = doc.nodes[body]
        .ifc_lines()
        .expect("body lines")
        .iter()
        .map(shodo::Line::block_size)
        .collect();
    let first_layout = doc.nodes[first].unrounded_layout;
    assert_eq!(first_layout.location.y, 0.0);
    assert_eq!(first_layout.size.height, heights[0]);
    // The second break is located relative to its span's box on the second
    // line; together they put it at the top of that line.
    let second_layout = doc.nodes[second].unrounded_layout;
    let span_y = doc.nodes[span].unrounded_layout.location.y;
    assert_eq!(span_y + second_layout.location.y, heights[0]);
    assert_eq!(second_layout.size.height, heights[1]);
    assert_eq!(second_layout.size.width, 0.0);
}
