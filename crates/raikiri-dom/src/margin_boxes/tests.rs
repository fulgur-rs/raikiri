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
        (16.0, FontFamilyName::generic("serif"))
    );
}

#[test]
fn inherited_margin_box_font_resolves_keywords_and_calc() {
    // `@page` and margin-box `<absolute-size>` keywords resolve against the
    // default 16px `medium`; a margin-box keyword does not scale the page's.
    // The page cascade has already computed the page keyword to px, while
    // the margin-box declaration still carries the keyword. A mixed calc
    // resolves against the page's size.
    use raikiri_style::property::AbsoluteFontSize;
    for (css, expected) in [
        (
            "@page { font-size: x-large; @top-left { content: 'x'; } }",
            AbsoluteFontSize::XLarge.px_for_medium(16.0),
        ),
        (
            "@page { font-size: x-large; @top-left { content: 'x'; font-size: small; } }",
            AbsoluteFontSize::Small.px_for_medium(16.0),
        ),
        (
            "@page { font-size: 20px; @top-left { content: 'x'; font-size: calc(50% + 2px); } }",
            12.0,
        ),
        (
            "html { line-height: 30px } @page { font-size: 20px; \
             @top-left { content: 'x'; font-size: calc(1rlh + 2px); } }",
            32.0,
        ),
        (
            "@page { font-size: 20px; line-height: 2; \
             @top-left { content: 'x'; font-size: calc(1lh + 2px); } }",
            42.0,
        ),
        (
            "@page { font-size: 20px; line-height: 25px; \
             @top-left { content: 'x'; font-size: calc(1lh + 2px); } }",
            27.0,
        ),
    ] {
        let mut document = Document::new();
        let html = document.append_element(
            Some(document.root_index()),
            "html",
            Style::default(),
            None::<&str>,
        );
        let style = document.append_element(Some(html), "style", Style::default(), None::<&str>);
        document.append_text(style, css);
        let rules = build_rule_tree(&document);
        let cascade = cascade(&document, &rules).expect("cascade Ok");
        let rule = cascade
            .page
            .cascade_margin_box(PageMarginBoxSlot::TopLeft)
            .expect("the @page fixture has a margin box");
        assert_eq!(
            inherited_margin_box_font(&document, &cascade, &cascade.page, &rule),
            (expected, FontFamilyName::generic("serif")),
            "{css}"
        );
    }
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
    MarginBoxSpec {
        slot: PageMarginBoxSlot::TopCenter,
        content: String::new(),
        deferred: Vec::new(),
        background: Some(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
        background_clip: VisualBox::BorderBox,
        background_layers: Vec::new(),
        background_layer_count: 1,
        background_image_lime: false,
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
        text_shadows: Vec::new(),
        text_style: text_style(16.0, &[]),
        alignment: StandaloneAlign::Start,
        vertical_align: MarginTextVerticalAlign::Top,
        running: None,
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
    spec.text_style.families = vec![FontFamilyName::generic("serif")];
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
    spec.text_style = text_style(10.0, &[FontFamilyName::named("Ahem")]);
    spec.text_style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    assert_eq!(margin_box_text_width(&doc, &spec, None), 20.0);
    assert_eq!(margin_box_intrinsic_height(&doc, &spec), 30.0);
}

#[test]
fn vertical_auto_width_counts_columns_wrapped_to_the_content_height() {
    let doc = engine_document();
    let mut spec = fixed_margin_spec();
    spec.content = "ab cd ef".into();
    spec.text_style = text_style(10.0, &[FontFamilyName::named("Ahem")]);
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
    long.text_style = text_style(10.0, &[FontFamilyName::named("Ahem")]);
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
            &MarginBoxNamedValues::new(),
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
    spec.text_style.families = vec![FontFamilyName::named("Ahem")];
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
fn margin_box_text_runs_carry_the_text_shadow() {
    let (doc, cascade) = page_cascade_fixture(
        "html { text-shadow: 1px 1px blue } \
         @page { margin: 40px; \
           @top-left { content: 'abc'; font-size: 10px; color: red; \
                       text-shadow: 0.2em 3px 2px, 1px 1px lime } \
           @top-right { content: 'abc' } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    let [left, right] = boxes.as_slice() else {
        panic!("two boxes: {boxes:?}");
    };
    let shadows = &left.text_runs()[0].shadows;
    assert_eq!(shadows.len(), 2);
    assert_eq!(shadows[0].offset, (2.0, 3.0), "em resolves against the box");
    assert_eq!(shadows[0].blur_radius, 2.0);
    assert_eq!(
        (shadows[0].color.r, shadows[0].color.b),
        (255, 0),
        "currentcolor is the box's text color"
    );
    assert_eq!(shadows[1].color.g, 255);
    let inherited = &right.text_runs()[0].shadows;
    assert_eq!(inherited.len(), 1, "inherited from the root element");
    assert_eq!(inherited[0].offset, (1.0, 1.0));
    assert_eq!(inherited[0].color.b, 255);
}

#[test]
fn a_margin_box_padding_calc_resolves_against_its_width_basis() {
    let origin = |padding: &str| {
        let (doc, cascade) = page_cascade_fixture(&format!(
            "@page {{ margin: 40px; \
               @top-left {{ content: 'abc'; font-family: Ahem; font-size: 10px; \
                           text-align: left; padding-left: {padding} }} }}"
        ));
        let boxes = layout_page(&doc, &cascade, small_page());
        let [head] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        head.text_runs()[0].origin.0
    };
    let none = origin("0px");
    let em = origin("1em") - none;
    let percent = origin("10%") - none;
    assert!(em > 0.0 && percent > 0.0);
    assert_eq!(origin("calc(10% + 1em)") - none, percent + em);
    assert_eq!(origin("calc(10% + 10px)") - none, percent + 10.0);
}

#[test]
fn margin_box_calc_rem_terms_use_the_root_font_size() {
    let origin = |declarations: &str| {
        let (doc, cascade) = page_cascade_fixture(&format!(
            "html {{ font-size: 20px }} @page {{ margin: 40px; \
               @top-left {{ content: 'abc'; font-family: Ahem; font-size: 10px; \
                           text-align: left; {declarations} }} }}"
        ));
        let boxes = layout_page(&doc, &cascade, small_page());
        let [head] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        head.text_runs()[0].origin.0
    };
    let none = origin("");
    let percent = origin("padding-left: 10%") - none;
    assert_eq!(
        origin("padding-left: calc(10% + 1rem)") - none,
        percent + 20.0
    );
    assert_eq!(
        origin("margin-left: calc(10% + 1rem)") - none,
        percent + 20.0
    );
    assert_eq!(
        origin("margin-left: calc(10% + 1em)") - none,
        percent + 10.0
    );
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
    text.text_style = text_style(10.0, &[FontFamilyName::named("Ahem")]);
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

#[test]
fn a_deferred_page_count_marks_its_placeholder() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; \
           @top-left { content: 'p.' counter(page) ' of ' counter(pages) ' (' \
                       counter(pages, upper-roman) ')'; font-family: Ahem; font-size: 10px } }",
    );
    let boxes = page_margin_boxes(
        &doc,
        &cascade,
        &cascade.page,
        small_page(),
        MarginBoxPageContext::new(0, 999, false).with_deferred_page_count(),
    );
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(head.content, "p.1 of 999 (CMXCIX)");
    let ranges: Vec<_> = head
        .deferred
        .iter()
        .map(|slot| slot.range.clone())
        .collect();
    assert_eq!(ranges, [7..10, 12..18]);
    assert!(
        head.deferred
            .iter()
            .all(|slot| slot.value == DeferredValue::PageCount)
    );
    assert_eq!(head.deferred[0].text(12), "12");
    assert_eq!(head.deferred[1].text(12), "XII");

    let runs = head.text_runs();
    let glyphs = head.deferred_glyphs();
    assert_eq!(glyphs.len(), 3 + 6);
    for glyph in &glyphs {
        let run = &runs[glyph.run];
        let shown = &run.text[run.glyphs[glyph.glyph].text_range.clone()];
        assert_eq!(
            &head.content[glyph.text_range.clone()],
            shown,
            "glyph text range of slot {}",
            glyph.slot
        );
        assert!(
            head.deferred[glyph.slot].range.start <= glyph.text_range.start
                && glyph.text_range.end <= head.deferred[glyph.slot].range.end
        );
        let expected = if glyph.slot == 0 { "9" } else { "CMXIV" };
        assert!(
            expected.contains(shown),
            "slot {} shows {shown:?}",
            glyph.slot
        );
    }
}

#[test]
fn a_deferred_page_count_in_a_linear_style_is_shown_in_decimal() {
    let (doc, cascade) = page_cascade_fixture(
        "@counter-style ticks { system: symbolic; symbols: 'x' } \
         @page { margin: 40px; \
           @top-left { content: counter(pages, ticks); font-family: Ahem; font-size: 10px } }",
    );
    let boxes = page_margin_boxes(
        &doc,
        &cascade,
        &cascade.page,
        small_page(),
        MarginBoxPageContext::new(0, 99_999, false).with_deferred_page_count(),
    );
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(head.content, "99999");
    assert_eq!(head.deferred[0].range, 0..5);
    assert_eq!(head.deferred[0].text(3), "xxx");
}

#[test]
fn an_empty_deferred_placeholder_is_shown_in_decimal() {
    let (doc, cascade) = page_cascade_fixture(
        "@counter-style pair { system: cyclic; symbols: '' 'X' } \
         @page { margin: 40px; \
           @top-left { content: 'of ' counter(pages, pair); font-family: Ahem; font-size: 10px } }",
    );
    let boxes = page_margin_boxes(
        &doc,
        &cascade,
        &cascade.page,
        small_page(),
        MarginBoxPageContext::new(0, 99_999, false).with_deferred_page_count(),
    );
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    assert_eq!(head.content, "of 99999");
    assert_eq!(head.deferred[0].range, 3..8);
    assert_eq!(head.deferred[0].text(2), "X");
    assert_eq!(head.deferred_glyphs().len(), 5);
}

#[test]
fn a_deferred_slot_formats_through_its_fallback_chain() {
    let (doc, cascade) = page_cascade_fixture(
        "@counter-style few { system: cyclic; symbols: 'a'; range: 1 2; fallback: ticks } \
         @counter-style ticks { system: symbolic; symbols: 'x' } \
         @counter-style unrelated { system: cyclic; symbols: 'u' } \
         @page { margin: 40px; \
           @top-left { content: counter(pages, few); font-family: Ahem; font-size: 10px } }",
    );
    let boxes = page_margin_boxes(
        &doc,
        &cascade,
        &cascade.page,
        small_page(),
        MarginBoxPageContext::new(0, 9, false).with_deferred_page_count(),
    );
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let slot = &head.deferred[0];
    assert_eq!(slot.text(2), "a");
    assert_eq!(slot.text(3), "xxx");
}

#[test]
fn a_known_page_count_has_no_deferred_slots() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; @top-left { content: counter(pages); font-family: Ahem } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    assert_eq!(boxes[0].content, "1");
    assert!(boxes[0].deferred.is_empty());
    assert!(boxes[0].deferred_glyphs().is_empty());
}

#[test]
fn margin_box_text_shadow_resolves_signed_and_root_relative_offsets() {
    let (doc, cascade) = page_cascade_fixture(
        "html { text-shadow: 1px 1px blue; line-height: 2 } \
         @page { margin: 40px; text-shadow: 4px 4px lime; \
           @top-left { content: 'abc'; font-size: 20px; \
                       text-shadow: -2px -3px red, 1rem 0, 1rlh 0 } \
           @top-right { content: 'abc'; text-shadow: inherit } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    let [left, right] = boxes.as_slice() else {
        panic!("two boxes: {boxes:?}");
    };
    let negative = &left.text_runs()[0].shadows;
    assert_eq!(negative.len(), 3);
    assert_eq!(negative[0].offset, (-2.0, -3.0));
    assert_eq!(
        negative[1].offset,
        (16.0, 0.0),
        "rem resolves against the root"
    );
    let inherited = &right.text_runs()[0].shadows;
    assert_eq!(inherited.len(), 1, "inherit takes the page context's value");
    assert_eq!(inherited[0].offset, (4.0, 4.0));
}

#[test]
fn margin_box_text_shadow_substitutes_variables() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; \
           @top-left { content: 'abc'; --shadow: 2px 3px red; text-shadow: var(--shadow) } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let shadows = &head.text_runs()[0].shadows;
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].offset, (2.0, 3.0));
    assert_eq!(shadows[0].color.r, 255);
}

#[test]
fn a_margin_box_color_takes_the_clip_of_the_bottom_layer() {
    let clip = |declarations: &str| {
        let (doc, cascade) = page_cascade_fixture(&format!(
            "@page {{ margin: 40px; @top-left {{ content: 'x'; {declarations} }} }}"
        ));
        let boxes = layout_page(&doc, &cascade, small_page());
        let [head] = boxes.as_slice() else {
            panic!("one box: {boxes:?}");
        };
        head.background_clip
    };
    assert_eq!(clip(""), VisualBox::BorderBox);
    assert_eq!(clip("background-clip: content-box"), VisualBox::ContentBox);
    // The clip list repeats to the number of image layers.
    assert_eq!(
        clip(
            "background-image: url(a.png), none, none; \
             background-clip: content-box, padding-box"
        ),
        VisualBox::ContentBox
    );
    assert_eq!(
        clip("background: url(a.png), red content-box"),
        VisualBox::ContentBox
    );
}

#[test]
fn margin_box_layers_below_the_painted_ones_are_dropped() {
    let mut images = vec!["none"; MAX_PAINTED_BACKGROUND_LAYERS];
    images[0] = "url(top.png)";
    images.push("url(under.png)");
    let (doc, cascade) = page_cascade_fixture(&format!(
        "@page {{ margin: 40px; @top-left {{ content: 'x'; background-image: {} }} }}",
        images.join(", ")
    ));
    let boxes = layout_page(&doc, &cascade, small_page());
    let [head] = boxes.as_slice() else {
        panic!("one box: {boxes:?}");
    };
    let urls: Vec<_> = head
        .background_layers
        .iter()
        .map(|layer| layer.url.as_str())
        .collect();
    assert_eq!(urls, ["top.png"]);
    assert_eq!(
        head.background_layer_count,
        MAX_PAINTED_BACKGROUND_LAYERS + 1
    );
}

#[test]
fn margin_box_text_shadow_takes_the_css_wide_keywords() {
    let (doc, cascade) = page_cascade_fixture(
        "@page { margin: 40px; text-shadow: 4px 4px lime; \
           @top-left { content: 'abc'; text-shadow: 1px 1px red; text-shadow: initial } \
           @top-right { content: 'abc'; text-shadow: 1px 1px red; text-shadow: unset } }",
    );
    let boxes = layout_page(&doc, &cascade, small_page());
    let [left, right] = boxes.as_slice() else {
        panic!("two boxes: {boxes:?}");
    };
    assert!(left.text_runs()[0].shadows.is_empty(), "initial is none");
    let unset = &right.text_runs()[0].shadows;
    assert_eq!(unset.len(), 1, "unset inherits the page context's value");
    assert_eq!(unset[0].offset, (4.0, 4.0));
}
