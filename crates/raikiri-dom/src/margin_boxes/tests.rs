use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

fn engine_document() -> Document {
    let mut doc = Document::new();
    let dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    ));
    let collection = crate::build_wpt_font_collection(dir).expect("the Ahem layer");
    doc.set_font_collection_with_limits(collection, shodo::limits::Limits::default());
    doc
}

/// Glyph positions of placed text, in the same way the built-in painter
/// draws standalone text.
fn glyph_positions(text: &MarginBoxText) -> Vec<(f32, f32)> {
    let (x, y) = text.origin();
    let shaped = text.shaped();
    let mut out = Vec::new();
    for (index, line) in shaped.lines().iter().enumerate() {
        let converter = shodo::geometry::PhysicalConverter::new(
            line.writing_mode(),
            line.used_direction(),
            shaped.container(),
        );
        for fragment in line.fragments() {
            let shodo::Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            for glyph in 0..run.glyphs().count() {
                let Some((gx, gy)) = run.glyph_origin(glyph) else {
                    continue;
                };
                let (px, py) =
                    converter.point(gx + shaped.hang_shift(index), gy + line.block_offset());
                out.push((x + px, y + py));
            }
        }
    }
    out
}

fn small_page() -> PageBox {
    let mut page_box = PageBox::new();
    page_box.width = 400.0;
    page_box.height = 300.0;
    page_box
}

fn page_cascade_fixture(css: &str) -> (Document, CascadeResult) {
    let mut doc = engine_document();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let head = doc.append_element(Some(html), "head", Style::default(), Some("display:none"));
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, css);
    doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    (doc, cascade)
}

fn layout_page(doc: &Document, cascade: &CascadeResult, page_box: PageBox) -> Vec<MarginBox> {
    page_margin_boxes(
        doc,
        cascade,
        &cascade.page,
        page_box,
        MarginBoxPageContext::new(0, 1, false),
    )
}

#[test]
fn inherited_margin_box_font_uses_root_computed_family() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(style, "@page { @top-left { content: 'x'; } }");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let rule = cascade
        .page
        .margin_boxes()
        .first()
        .expect("the @page fixture has a margin box");

    assert_eq!(
        inherited_margin_box_font(&document, &cascade, &cascade.page, rule),
        (16.0, "serif".to_owned())
    );
}

fn margin_row_margins(top: f32) -> PageMargins {
    PageMargins {
        top,
        right: 10.0,
        bottom: 10.0,
        left: 10.0,
    }
}

fn fixed_margin_spec() -> MarginBoxSpec {
    let initial = ComputedValues::initial();
    MarginBoxSpec {
        slot: PageMarginBoxSlot::TopCenter,
        content: String::new(),
        background: Some(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
        background_image_url: None,
        background_image_lime: false,
        background_size: initial.background_size,
        background_position: initial.background_position,
        background_repeat: initial.background_repeat,
        background_origin: initial.background_origin,
        background_clip: initial.background_clip,
        content_image_lime: false,
        border_top: None,
        border_right: None,
        border_bottom: None,
        border_left: None,
        margin_auto: [false; 4],
        margin: [0.0; 4],
        padding: [0.0; 4],
        width: Some(100.0),
        height: None,
        text_color: CssColor::BLACK,
        text_style: text_style(16.0, ""),
        alignment: StandaloneAlign::Start,
        vertical_align: MarginTextVerticalAlign::Top,
    }
}

fn layout_margin_row(
    document: &Document,
    specs: &[MarginBoxSpec],
    top: bool,
    page_width: f32,
    page_height: f32,
    margins: PageMargins,
) -> Vec<MarginBox> {
    let mut out = Vec::new();
    layout_horizontal_margin_boxes(
        document,
        specs,
        top,
        page_width,
        page_height,
        margins,
        &mut out,
    );
    out
}

#[test]
fn margin_row_without_specs_lays_out_nothing() {
    let boxes = layout_margin_row(
        &Document::new(),
        &[],
        true,
        PageBox::A4.width,
        PageBox::A4.height,
        margin_row_margins(10.0),
    );
    assert!(boxes.is_empty());
}

#[test]
fn margin_row_without_row_height_lays_out_nothing() {
    let boxes = layout_margin_row(
        &Document::new(),
        &[fixed_margin_spec()],
        true,
        PageBox::A4.width,
        PageBox::A4.height,
        margin_row_margins(0.0),
    );
    assert!(boxes.is_empty());
}

#[test]
fn margin_row_lays_out_fixed_width_background() {
    let boxes = layout_margin_row(
        &Document::new(),
        &[fixed_margin_spec()],
        true,
        PageBox::A4.width,
        PageBox::A4.height,
        margin_row_margins(10.0),
    );
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].rect, PaintRect::new(10.0, 0.0, 100.0, 10.0));
    assert_eq!(boxes[0].background_color, fixed_margin_spec().background);
    assert!(boxes[0].text.is_none());
}

#[test]
fn a_margin_box_width_follows_the_document_font() {
    // "serif" resolves to Ahem in the document layer (30px for "abc" at 10px);
    // the system serif font is nowhere near that.
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc".to_owned();
    spec.text_style.families = vec!["serif".to_owned()];
    spec.text_style.font_size = 10.0;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 30.0);
    assert_eq!(doc.standalone_text_calls(), 1);
    // Vertical content width is the block advance of one column.
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 10.0);
    assert_eq!(doc.standalone_text_calls(), 2);
}

#[test]
fn vertical_margin_box_intrinsics_use_physical_axes() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc\ndef".into();
    spec.text_style = text_style(10.0, "Ahem");
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 20.0);
    assert_eq!(margin_box_intrinsic_height(&doc, &spec), 30.0);
}

#[test]
fn vertical_auto_width_counts_columns_wrapped_to_the_content_height() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "ab cd ef".into();
    spec.text_style = text_style(10.0, "Ahem");
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    spec.height = Some(25.0);
    assert_eq!(margin_box_text_width(&doc, &spec, None), 30.0);
}

#[test]
fn vertical_margin_row_distributes_auto_width_using_wrapped_columns() {
    let doc = engine_document();
    let mut long = fixed_margin_spec();
    long.slot = PageMarginBoxSlot::TopLeft;
    long.content = "ab cd ef".into();
    long.text_style = text_style(10.0, "Ahem");
    long.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    long.width = None;
    let mut short = long.clone();
    short.slot = PageMarginBoxSlot::TopRight;
    short.content = "ab".into();
    let widths: Vec<_> = layout_margin_row(
        &doc,
        &[long, short],
        true,
        220.0,
        200.0,
        margin_row_margins(25.0),
    )
    .iter()
    .map(|margin_box| margin_box.rect.width)
    .collect();
    assert_eq!(widths, [150.0, 50.0]);
}

#[test]
fn margin_box_layer_precedence_reaches_the_margin_box_spec() {
    for (sheets, expected) in [
        (
            [
                "@layer a,b; @layer b{@page{@top-left{color:blue}}}",
                "@layer a{@page{@top-left{color:red}}}",
            ],
            raikiri_style::CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        ),
        (
            [
                "@layer a,b; @layer a{@page{@top-left{color:red!important}}}",
                "@layer b{@page{@top-left{color:blue!important}}} @page{@top-left{color:green!important}}",
            ],
            raikiri_style::CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
        ),
    ] {
        let mut tree = raikiri_style::RuleTree::empty();
        for sheet in sheets {
            tree.add_stylesheet(sheet, raikiri_style::Origin::Author);
        }
        let page = raikiri_style::cascade_page(
            &tree,
            &raikiri_style::PageContextQuery::default(),
            raikiri_style::PageInheritance::LegacyInitialValues,
        );
        let rule = margin_box_rule(&page, PageMarginBoxSlot::TopLeft).unwrap();
        assert_eq!(
            margin_box_property(&rule, PropertyKey::Color),
            Some(&PropertyValue::Color(expected))
        );
    }
}

#[test]
fn margin_box_flow_inherits_from_root_and_page_and_allows_local_override() {
    use shodo::geometry::{Direction, WritingMode as Mode};
    use shodo::style::TextOrientation as Orientation;
    for (page, local, mode, orientation, direction) in [
        (
            "",
            "",
            Mode::VerticalRl,
            Orientation::Upright,
            Direction::Rtl,
        ),
        (
            "writing-mode:sideways-lr;text-orientation:sideways;direction:ltr;",
            "",
            Mode::SidewaysLr,
            Orientation::Sideways,
            Direction::Ltr,
        ),
        (
            "writing-mode:sideways-lr;",
            "writing-mode:vertical-lr;text-orientation:mixed;direction:ltr;",
            Mode::VerticalLr,
            Orientation::Mixed,
            Direction::Ltr,
        ),
        (
            "writing-mode:vertical-rl;",
            "writing-mode:horizontal-tb;",
            Mode::HorizontalTb,
            Orientation::Upright,
            Direction::Rtl,
        ),
        (
            "",
            "writing-mode:sideways-rl;",
            Mode::SidewaysRl,
            Orientation::Upright,
            Direction::Rtl,
        ),
    ] {
        let mut doc = engine_document();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some("display:block;writing-mode:vertical-rl;text-orientation:upright;direction:rtl"),
        );
        let node = doc.append_element(Some(html), "style", Style::default(), Some("display:none"));
        doc.append_text(node, format!("@page {{ {page} @top-left {{ content:'ab';font-family:Ahem;font-size:10px;text-align:end; {local} }} }}"));
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        let rule = cascade.page.margin_boxes().first().unwrap();
        let spec = margin_box_spec(
            &doc,
            &cascade,
            &cascade.page,
            rule,
            100.0,
            40.0,
            MarginBoxPageContext::new(0, 1, false),
        )
        .unwrap();
        assert_eq!(spec.text_style.writing_mode, mode, "{page} {local}");
        assert_eq!(spec.text_style.text_orientation, orientation);
        assert_eq!(spec.text_style.direction, direction);
        assert_eq!(spec.alignment, StandaloneAlign::End);
        let placed = place_margin_box(&doc, &spec, PaintRect::new(0.0, 0.0, 100.0, 40.0))
            .expect("the box has an area");
        let positions = glyph_positions(placed.text.as_ref().expect("the box has text"));
        assert_eq!(positions.len(), 2);
        if mode.is_vertical() {
            assert_eq!(positions[0].0, positions[1].0);
            assert_eq!((positions[1].1 - positions[0].1).abs(), 10.0);
        } else {
            assert_eq!(positions[0].1, positions[1].1);
            assert_eq!((positions[1].0 - positions[0].0).abs(), 10.0);
        }
    }
}

#[test]
fn a_vertical_writing_margin_box_advances_glyphs_down_the_column() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "abc".to_owned();
    spec.text_style.families = vec!["Ahem".to_owned()];
    spec.text_style.font_size = 10.0;
    let glyphs = |spec: &MarginBoxSpec| -> Vec<(f32, f32)> {
        let placed = place_margin_box(&doc, spec, PaintRect::new(0.0, 0.0, 200.0, 40.0))
            .expect("the box has an area");
        glyph_positions(placed.text.as_ref().expect("the box has text"))
    };
    let horizontal = glyphs(&spec);
    assert_eq!(doc.standalone_text_calls(), 1);
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    let vertical = glyphs(&spec);
    assert_eq!(doc.standalone_text_calls(), 2, "the engine drew it");
    assert!(!vertical.is_empty());
    assert_ne!(vertical, horizontal);
    assert_eq!(vertical.len(), 3);
    assert_eq!(vertical[0].0, vertical[1].0);
    assert_eq!(vertical[1].1 - vertical[0].1, 10.0);
}

mod css_wide_tests;

#[test]
fn page_margin_boxes_resolve_counters_per_page_in_drawing_order() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; \
           @bottom-center { content: 'p.' counter(page) ' / ' counter(pages) } \
           @top-left { content: 'Head'; font-family: Ahem; font-size: 10px; \
                       background-color: blue; border-bottom: 2px solid red } \
           @top-right { content: none; background-color: green } }",
    );
    let page_box = small_page();
    let boxes = page_margin_boxes(
        &doc,
        &cascade,
        &cascade.page,
        page_box,
        MarginBoxPageContext::new(2, 5, false),
    );
    let slots: Vec<_> = boxes.iter().map(|margin_box| margin_box.slot).collect();
    // `content: none` suppresses the box; the top row comes first.
    assert_eq!(
        slots,
        [PageMarginBoxSlot::TopLeft, PageMarginBoxSlot::BottomCenter]
    );
    let head = &boxes[0];
    assert_eq!(head.content, "Head");
    assert_eq!(head.rect.y, 0.0);
    assert_eq!(head.rect.height, 40.0);
    assert_eq!(head.background_color.map(|color| color.b), Some(255));
    assert_eq!(
        head.borders[2].map(|border| border.width),
        Some(2.0),
        "solid bottom border"
    );
    assert_eq!(head.border_widths().bottom, 2.0);
    assert_eq!(boxes[1].content, "p.3 / 5");
    assert!(boxes[1].rect.y >= 260.0);
}

#[test]
fn margin_box_text_runs_sit_inside_the_box() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; \
           @top-left { content: 'abc'; font-family: Ahem; font-size: 10px; color: red; \
                       vertical-align: middle } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let runs = head.text_runs();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.source, RunSource::MarginBox(PageMarginBoxSlot::TopLeft));
    assert_eq!(run.text, "abc");
    assert_eq!(run.glyphs.len(), 3);
    assert_eq!(run.color.r, 255);
    assert_eq!(run.font_size, 10.0);
    // The 10px line is centered in the 40px strip: Ahem's baseline is 8px
    // below the line top.
    assert_eq!(run.origin, (40.0, 15.0 + 8.0));
    assert_eq!(run.advance, 30.0);
}

#[test]
fn vertical_margin_box_text_has_no_runs() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; \
           @top-left { content: 'abc'; font-family: Ahem; font-size: 10px; \
                       writing-mode: vertical-rl } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].text.is_some());
    assert!(boxes[0].text_runs().is_empty());
}

#[test]
fn zero_page_margins_have_no_margin_boxes() {
    let (doc, cascade) = page_cascade_fixture("@page { margin: 0; @top-left { content: 'abc' } }");
    assert!(layout_page(&doc, &cascade, small_page()).is_empty());
}

#[test]
fn margin_box_text_is_debuggable_and_borders_clamp_to_the_box() {
    let mut spec = fixed_margin_spec();
    spec.border_top = Some((80.0, CssColor::BLACK));
    let placed = place_margin_box(
        &Document::new(),
        &spec,
        PaintRect::new(0.0, 0.0, 100.0, 20.0),
    )
    .expect("the box has an area");
    assert_eq!(placed.borders[0].map(|border| border.width), Some(20.0));
    assert!(
        place_margin_box(&Document::new(), &spec, PaintRect::new(0.0, 0.0, 0.0, 20.0)).is_none()
    );
    let mut text = fixed_margin_spec();
    text.content = "abc".into();
    text.text_style = text_style(10.0, "Ahem");
    let placed = place_margin_box(
        &engine_document(),
        &text,
        PaintRect::new(0.0, 0.0, 100.0, 20.0),
    )
    .unwrap();
    let debug = format!("{:?}", placed.text.unwrap());
    assert!(debug.contains("lines: 1"), "{debug}");
}

mod layout_tests;
