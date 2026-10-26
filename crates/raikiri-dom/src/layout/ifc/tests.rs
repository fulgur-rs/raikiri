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
use crate::layout::test_support::{
    ahem_paragraph, ahem_paragraph_with, ifc_ahem_fonts, page_box_800x600,
};
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
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    doc.append_text(body, " ");
    let replaced = doc.append_element(Some(body), tag, Some("width:2px;height:2px"));
    doc.append_text(body, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    doc.nodes[replaced].layout.location.y
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
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let parent = if let Some(css) = wrapper_css {
        doc.append_element(Some(body), "span", Some(css))
    } else {
        body
    };
    let image = doc.append_element(
        Some(parent),
        "img",
        Some(&format!(
            "width:2px;height:2px;vertical-align:{vertical_align}"
        )),
    );
    if second_line_text {
        doc.append_element(Some(body), "br", None::<&str>);
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
    (doc.nodes[image].layout.location.y, heights)
}

fn mixed_image_line_geometry(
    mode: raikiri_traits::QuirksMode,
    wrapped: bool,
    vertical_aligns: &[&str],
) -> (Vec<f32>, Vec<(f32, f32)>) {
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let mut images = Vec::new();
    for (line, vertical_align) in vertical_aligns.iter().enumerate() {
        if matches!(line, 1 | 3) {
            doc.append_text(body, "x");
        }
        let parent = if wrapped {
            doc.append_element(Some(body), "span", None::<&str>)
        } else {
            body
        };
        let image = doc.append_element(
            Some(parent),
            "img",
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
            doc.append_element(Some(body), "br", None::<&str>);
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
    let positions = images
        .into_iter()
        .map(|image| doc.nodes[image].layout.location.y)
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
    let mut doc = Document::new();
    doc.set_quirks_mode(mode);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    let outer = doc.append_element(Some(body), "span", Some("line-height:40px"));
    let inner = doc.append_element(Some(outer), "b", Some(inner_css));
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
    let mut doc = Document::new();
    doc.set_quirks_mode(raikiri_traits::QuirksMode::Quirks);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", None::<&str>);
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(style, "span::before { content: \"x\"; line-height: 10px; }");
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    doc.append_element(Some(body), "span", Some("line-height:40px"));
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
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-size:20px;line-height:20px"),
    );
    doc.append_text(body, "aa");
    let first = doc.append_element(Some(body), "br", None::<&str>);
    doc.append_text(body, "bbbb");
    let span = doc.append_element(Some(body), "span", Some("line-height:40px"));
    let second = doc.append_element(Some(span), "br", None::<&str>);
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
    let first_layout = doc.nodes[first].layout;
    assert_eq!(first_layout.location.y, 0.0);
    assert_eq!(first_layout.size.height, heights[0]);
    // The second break is located relative to its span's box on the second
    // line; together they put it at the top of that line.
    let second_layout = doc.nodes[second].layout;
    let span_y = doc.nodes[span].layout.location.y;
    assert_eq!(span_y + second_layout.location.y, heights[0]);
    assert_eq!(second_layout.size.height, heights[1]);
    assert_eq!(second_layout.size.width, 0.0);
}

#[test]
fn quirks_mode_a_line_break_uses_the_box_generating_ancestor_of_a_contents_body() {
    use raikiri_traits::QuirksMode;
    for (tag, display, expected) in [
        ("body", "contents", [20.0, 10.0]),
        ("span", "contents", [20.0, 10.0]),
        ("body", "block", [40.0, 20.0]),
    ] {
        let mut doc = Document::new();
        doc.set_quirks_mode(QuirksMode::Quirks);
        let html = doc.append_element(
            Some(0),
            "html",
            Some("display:block;font-size:10px;line-height:10px"),
        );
        // A contents element has no strut. Its paragraph and break use html's
        // 10px strut, while a block body contributes its own 20px strut.
        let body = doc.append_element(
            Some(html),
            tag,
            Some(format!("display:{display};margin:0;font-size:20px;line-height:20px").as_str()),
        );
        // The layout API currently requires a body, even for the span control.
        if tag != "body" {
            doc.append_element(Some(html), "body", Some("display:none"));
        }
        doc.append_element(Some(body), "br", Some("line-height:2"));
        doc.append_text(body, "z");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        layout_single_page(&mut doc, &cascade, raikiri_traits::PageBox::A4).expect("layout");
        let owner = if display == "block" { body } else { html };
        let heights: Vec<f32> = doc.nodes[owner]
            .ifc_lines()
            .expect("paragraph lines")
            .iter()
            .map(shodo::Line::block_size)
            .collect();
        assert_eq!(heights, expected, "{tag} {display}");
    }
}

fn intrinsic_measurements(doc: &Document) -> usize {
    doc.ifc
        .as_ref()
        .map_or(0, |state| state.intrinsic_measurements)
}

#[test]
fn a_stretched_horizontal_paragraph_is_not_measured_for_intrinsic_widths() {
    // A block-level paragraph is stretched to its containing block's definite
    // width, so its min- and max-content widths never decide its width.
    let (mut doc, cascade, root) = ahem_paragraph("AAAA BBBB CCCC", "width:60px");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(intrinsic_measurements(&doc), 0);
    let lines = doc.nodes[root].ifc_lines().expect("lines");
    assert_eq!(lines.len(), 3, "breaks at the 60px width");
    assert_eq!(doc.nodes[root].layout.size.width, 60.0);
}

#[test]
fn a_shrink_to_fit_paragraph_is_still_measured_for_intrinsic_widths() {
    for css in ["float:left", "display:inline-block", "position:absolute"] {
        let (mut doc, cascade, root) = ahem_paragraph("AAAA BBBB", css);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert!(intrinsic_measurements(&doc) > 0, "{css}");
        assert_eq!(
            doc.nodes[root].layout.size.width, 90.0,
            "{css} shrinks to its max-content width"
        );
    }
}

/// `depth` shrink-to-fit boxes of the kind `display`, each inside the one
/// before and each smaller than it, so that the widths a parent tries on a box
/// differ. Returns the document, its cascade and the outermost box.
fn nested_boxes(depth: usize, display: &str) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut outermost = None;
    let (doc, cascade, _) = ahem_paragraph_with("", |doc, root| {
        let mut parent = root;
        for _ in 0..depth {
            let span = doc.append_element(
                Some(parent),
                "span",
                Some(format!("display:{display};font-size:calc(1em / 1.1)").as_str()),
            );
            doc.append_text(span, "X");
            outermost.get_or_insert(span);
            parent = span;
        }
    });
    (doc, cascade, outermost.expect("a box"))
}

#[test]
fn nested_shrink_to_fit_boxes_are_measured_a_few_times_each() {
    // A box is asked for its intrinsic widths at every width its parent tries,
    // and each time it used to measure its content again, which asks the boxes
    // inside it the same: the scans of paragraphs grew by a factor of 2.6 with
    // every level (30240 at ten levels, about 200000 at twelve). They are kept
    // now, and the count follows the depth.
    //
    // A flex container is measured under a probe of its own, and a container
    // with a `max-width` or items with a length as their flex basis under
    // overrides of the style as well. Those used to move the cache generation,
    // which made every answer kept so far unreadable and multiplied the work
    // by four (five with `flex-wrap`, two and a bit for the basis) per level,
    // so their chains are kept short enough for a regression to fail rather
    // than hang.
    for (display, depth) in [
        ("inline-block", 12),
        ("inline-flex", 8),
        ("inline-flex;max-width:1000px", 8),
        ("inline-flex;max-width:50%", 8),
        ("inline-flex;max-width:1000px;flex-wrap:wrap", 7),
        ("inline-flex;flex:1 0 1px", 8),
        ("inline-flex;flex:0 1 1px", 8),
        ("inline-grid", 12),
    ] {
        let (mut doc, cascade, outermost) = nested_boxes(depth, display);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert!(
            intrinsic_measurements(&doc) < 40 * depth,
            "{display}: {} scans",
            intrinsic_measurements(&doc)
        );
        // Each box holds one `X`, as wide as its font size, and the box inside
        // it: side by side in a line or a row, one under the other in a grid,
        // which is as wide as the `X` then. An item that cannot grow adds at
        // most its flex basis of 1px for the box inside it.
        let mut widths = (1..=depth).map(|level| 10.0 / 1.1f32.powi(level as i32));
        let expected: f32 = match display {
            "inline-grid" => widths.next().expect("a box"),
            "inline-flex;flex:0 1 1px" => widths.next().expect("a box") + 1.0,
            _ => widths.sum(),
        };
        let width = doc.nodes[outermost].layout.size.width;
        assert!(
            (width - expected).abs() < 0.2,
            "{display}: the outermost box is {width} wide, not {expected}"
        );
    }
}

#[test]
fn nested_vertical_boxes_are_measured_a_few_times_each() {
    // A vertical box with an auto block size starts from the intrinsic extent
    // of its lines, which is not its block extent, so it laid its boxes out a
    // second time at the extent it found. Every level did, and the scans of
    // paragraphs doubled with each level of vertical boxes nested in each
    // other (49136 at fourteen levels). The extent that a box settled on is
    // the first try now, and the count follows the depth.
    let depth = 14;
    for display in ["block", "inline-block"] {
        let mut outermost = None;
        let (mut doc, cascade, root) =
            ahem_paragraph_with("writing-mode:vertical-rl", |doc, root| {
                let mut parent = root;
                for _ in 0..depth - 1 {
                    parent = doc.append_element(
                        Some(parent),
                        "div",
                        Some(format!("display:{display};writing-mode:vertical-rl").as_str()),
                    );
                    outermost.get_or_insert(parent);
                }
                doc.append_text(parent, "X");
            });
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert!(
            intrinsic_measurements(&doc) < 40 * depth,
            "{display}: {} scans",
            intrinsic_measurements(&doc)
        );
        // Every box holds one `X` in a column, so each is a column wide and as
        // tall as the `X`, and the boxes inside do not widen it.
        for node in [root, outermost.expect("a box")] {
            let size = doc.nodes[node].layout.size;
            assert_eq!(
                (size.width, size.height),
                (10.0, 10.0),
                "{display}: box {node}"
            );
        }
    }
}

#[test]
fn a_stretched_vertical_paragraph_is_still_measured_for_intrinsic_widths() {
    // In vertical flow the intrinsic widths still size the block extent.
    let (mut doc, cascade, _root) =
        ahem_paragraph("AAAA BBBB", "writing-mode:vertical-rl;height:60px");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(intrinsic_measurements(&doc) > 0);
}

#[test]
fn percentage_heights_in_a_paragraph_resolve_against_its_definite_height() {
    // CSS 2.1 §10.5: a percentage height resolves against the containing
    // block's height when that height does not depend on content, and
    // behaves as `auto` otherwise. The paragraph is that containing block
    // for its floats, atomic inlines and block children.
    let cases = [
        ("float:left;width:10px;height:50%", 50.0),
        ("display:inline-block;width:10px;height:50%", 50.0),
        ("display:block;height:25%", 25.0),
    ];
    for (child_css, expected) in cases {
        for (root_css, height) in [
            ("height:100px", expected),
            // A border-box height leaves the padding out of the basis.
            ("box-sizing:border-box;height:120px;padding:10px", expected),
            // `max-height` clamps the basis too.
            ("height:200px;max-height:100px", expected),
            // A horizontal scrollbar's gutter, when there is one, leaves the
            // basis too.
            ("height:100px;overflow-x:scroll", expected),
            // The basis is the physical height in a vertical writing mode
            // too.
            ("writing-mode:vertical-rl;height:100px", expected),
            ("", 10.0),
            // The body's auto height is indefinite, so percentages in the
            // paragraph also behave as auto.
            ("height:50%", 10.0),
        ] {
            let mut child = None;
            let (mut doc, cascade, _root) = ahem_paragraph_with(root_css, |doc, root| {
                doc.append_text(root, "AA");
                let id = doc.append_element(Some(root), "div", Some(child_css));
                doc.append_text(id, "B");
                child = Some(id);
            });
            layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
            let child = child.expect("child appended");
            assert_eq!(
                doc.nodes[child].layout.size.height, height,
                "{child_css} in a paragraph with {root_css:?}"
            );
        }
    }
}

/// An inline-block paragraph with `height:100px;{max_css}` inside an
/// auto-height wrapper, holding a `height:50%` float. Returns the
/// paragraph's and the float's heights.
fn inline_block_paragraph_in_an_auto_wrapper(max_css: &str) -> (f32, f32) {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
    let root = doc.append_element(
        Some(wrapper),
        "div",
        Some(&format!(
            "display:inline-block;width:100px;font-family:Ahem;font-size:10px;line-height:10px;height:100px;{max_css}"
        )),
    );
    doc.append_text(root, "AA");
    let child = doc.append_element(Some(root), "div", Some("float:left;width:10px;height:50%"));
    doc.append_text(child, "B");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (
        doc.nodes[root].layout.size.height,
        doc.nodes[child].layout.size.height,
    )
}

#[test]
fn a_percentage_max_height_against_an_indefinite_block_leaves_the_basis_alone() {
    // CSS 2.1 §10.7: the paragraph's percentage `max-height` cannot resolve
    // inside an auto-height wrapper, so it neither clamps the paragraph nor
    // the basis of its children's percentage heights. A length bound still
    // clamps both.
    assert_eq!(
        inline_block_paragraph_in_an_auto_wrapper("max-height:50%"),
        (100.0, 50.0)
    );
    assert_eq!(
        inline_block_paragraph_in_an_auto_wrapper("max-height:60px"),
        (60.0, 30.0)
    );
}
