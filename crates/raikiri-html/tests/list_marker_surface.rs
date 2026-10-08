//! List markers as positioned text consumed by an independent painter.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, RunSource, WarningKind, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, extra: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}\
         ol,ul{{margin:0;padding:0 0 0 50px}}li{{margin:0;padding:0}}\
         {extra}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

#[test]
fn outside_decimal_markers_have_document_fonts_positions_and_paint_events() {
    let document = lay_out("<ol><li>A</li><li>B</li></ol>", "");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let markers: Vec<_> = runs
        .iter()
        .filter(|run| {
            matches!(run.source, RunSource::Generated(_, GeneratedKind::Marker))
                && !run.text.trim().is_empty()
        })
        .collect();
    assert_eq!(markers.len(), 2);
    assert_eq!(markers[0].text.trim_end(), "1.");
    assert_eq!(markers[1].text.trim_end(), "2.");
    let body = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Text(_)))
        .unwrap();
    assert_eq!(markers[0].font.id, body.font.id);
    // Ahem gives each non-space glyph a 10px advance: "1." is 20px wide.
    assert_eq!(markers[0].origin.0, 26.0);
    assert_eq!(markers[1].origin.0, 26.0);
    assert_ne!(markers[0].line, body.line);
    assert!(markers.iter().all(|run| run.is_standalone_marker()));
    let events = page.paint_order_for_text_runs(&runs);
    for marker in markers {
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, PaintEvent::TextLine(line) if *line == marker.line))
                .count(),
            1
        );
    }
}

#[test]
fn inside_markers_are_not_duplicated_by_the_standalone_projection() {
    let document = lay_out(
        "<ol><li>A</li><li>B</li></ol>",
        "li{list-style-position:inside}",
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let markers: Vec<_> = runs
        .iter()
        .filter(|run| {
            matches!(run.source, RunSource::Generated(_, GeneratedKind::Marker))
                && !run.text.trim().is_empty()
        })
        .collect();
    assert_eq!(markers.len(), 2);
    assert_eq!(markers[0].text.trim_end(), "1.");
    assert_eq!(markers[1].text.trim_end(), "2.");
    assert!(markers.iter().all(|run| !run.is_standalone_marker()));
    let events = page.paint_order_for_text_runs(&runs);
    for marker in markers {
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, PaintEvent::TextLine(line) if *line == marker.line))
                .count(),
            1
        );
    }
}

#[test]
fn empty_list_items_still_expose_their_number() {
    let document = lay_out("<ol><li></li></ol>", "");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let markers: Vec<_> = runs
        .iter()
        .filter(|run| {
            matches!(run.source, RunSource::Generated(_, GeneratedKind::Marker))
                && !run.text.trim().is_empty()
        })
        .collect();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].text.trim_end(), "1.");
    assert_eq!(markers[0].origin.0, 26.0);
    assert!(
        page.paint_order_for_text_runs(&runs)
            .iter()
            .any(|event| matches!(event, PaintEvent::TextLine(line) if *line == markers[0].line))
    );
}

#[test]
fn a_marker_is_not_repeated_on_continuations_of_a_tall_list_item() {
    let document = lay_out(
        "<ol><li>A</li><li>B</li></ol>",
        "li:first-child{height:220px}",
    );
    assert!(document.page_count() >= 3);
    let numbers: Vec<Vec<_>> = document
        .pages()
        .map(|page| {
            page.text_runs()
                .into_iter()
                .filter(|run| {
                    matches!(run.source, RunSource::Generated(_, GeneratedKind::Marker))
                        && !run.text.trim().is_empty()
                })
                .map(|run| run.text.trim_end().to_owned())
                .collect()
        })
        .collect();
    assert_eq!(numbers, vec![vec!["1."], vec![], vec!["2."]]);
}

#[test]
fn nested_and_wrapped_items_keep_one_marker_at_each_list_gutter() {
    let document = lay_out(
        "<ol><li>ABCD<ul><li>E</li></ul></li><li>F</li></ol>",
        "li{width:20px;word-break:break-all}ul{list-style-type:disc}",
    );
    let markers: Vec<_> = document
        .pages()
        .flat_map(|page| page.text_runs())
        .filter(|run| run.is_standalone_marker() && !run.text.trim().is_empty())
        .map(|run| (run.text.trim_end().to_owned(), run.origin.0))
        .collect();
    assert_eq!(
        markers,
        vec![("1.".into(), 26.0), ("•".into(), 86.0), ("2.".into(), 26.0)]
    );
}

#[test]
fn standalone_marker_precedes_own_clip_and_stays_in_opacity_group() {
    let document = lay_out("<ol><li>A</li></ol>", "li{overflow:hidden;opacity:.5}");
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let marker = runs.iter().find(|run| run.is_standalone_marker()).unwrap();
    let events = page.paint_order_for_text_runs(&runs);
    let marker_index = events
        .iter()
        .position(|event| matches!(event,PaintEvent::TextLine(line) if *line == marker.line))
        .unwrap();
    let opacity = events
        .iter()
        .position(|event| matches!(event,PaintEvent::PushOpacity(value) if *value==0.5))
        .unwrap();
    let clip = events
        .iter()
        .position(|event| matches!(event, PaintEvent::PushClip(_, _)))
        .unwrap();
    let pop = events
        .iter()
        .position(|event| matches!(event, PaintEvent::PopOpacity))
        .unwrap();
    assert!(opacity < marker_index && marker_index < clip && clip < pop);
}

#[test]
fn suppressed_markers_have_no_runs_or_events() {
    for css in [
        "li{list-style-type:none}",
        "li::marker{content:''}",
        "li{visibility:hidden}",
        "li::marker{content:' '}",
        "li{width:0;height:0}",
    ] {
        let document = lay_out("<ul><li>A</li></ul>", css);
        assert!(
            !document
                .pages()
                .flat_map(|page| page.text_runs())
                .any(|run| run.is_standalone_marker())
        );
    }
}

#[test]
fn a_zero_sized_empty_item_still_exposes_its_marker() {
    let document = lay_out("<ol><li></li></ol>", "li{width:0;height:0}");
    assert!(
        document
            .pages()
            .flat_map(|page| page.text_runs())
            .any(|run| run.is_standalone_marker() && run.text.trim_end() == "1.")
    );
}

#[test]
fn resource_dependent_image_markers_are_excluded_but_authored_text_wins() {
    let document = lay_out(
        "<ul><li>A</li></ul>",
        "li{list-style-image:url(https://example.invalid/marker.png)}",
    );
    assert!(
        !document
            .pages()
            .flat_map(|page| page.text_runs())
            .any(|run| run.is_standalone_marker())
    );
    let document = lay_out(
        "<ul><li>A</li></ul>",
        "li{list-style-image:url(https://example.invalid/marker.png)}li::marker{content:'X'}",
    );
    assert!(
        document
            .pages()
            .flat_map(|page| page.text_runs())
            .any(|run| run.is_standalone_marker() && run.text == "X")
    );
}

#[test]
fn right_to_left_marker_glyphs_keep_visual_order_and_byte_ranges() {
    let document = lay_out("<ul><li>A</li></ul>", "li::marker{content:'אב'}");
    let runs = document.page(0).unwrap().text_runs();
    let run = runs.iter().find(|run| run.is_standalone_marker()).unwrap();
    assert_eq!(run.text, "אב");
    assert_eq!(
        run.glyphs
            .iter()
            .map(|glyph| glyph.text_range.clone())
            .collect::<Vec<_>>(),
        vec![2..4, 0..2]
    );
    let mut pen = run.origin.0;
    let positions: Vec<_> = run
        .glyphs
        .iter()
        .map(|glyph| {
            let x = pen + glyph.x_offset;
            pen += glyph.advance;
            x
        })
        .collect();
    assert!(positions[0] <= positions[1]);
}

#[test]
fn marker_with_negative_item_top_is_retained_on_the_first_page() {
    let document = lay_out("<ol><li>A</li></ol>", "li{margin-top:-5px}");
    let runs = document.page(0).unwrap().text_runs();
    assert!(
        runs.iter()
            .any(|run| run.is_standalone_marker() && run.text.trim_end() == "1.")
    );
}

#[test]
fn empty_item_at_the_last_page_boundary_keeps_its_marker() {
    let document = lay_out("<div style='height:100px'></div><ol><li></li></ol>", "");
    let markers: Vec<_> = document
        .pages()
        .flat_map(|page| page.text_runs())
        .filter(|run| run.is_standalone_marker())
        .collect();
    assert_eq!(markers.len(), 1);
}

#[test]
fn transformed_item_reports_one_omission_for_body_and_marker() {
    let document = lay_out("<ol><li>A</li></ol>", "li{transform:translateX(10px)}");
    assert!(
        !document
            .pages()
            .flat_map(|page| page.text_runs())
            .any(|run| run.is_standalone_marker())
    );
    assert_eq!(
        document
            .warnings()
            .iter()
            .filter(|warning| matches!(warning.kind, WarningKind::TextRunsOmitted))
            .count(),
        1
    );
}

#[test]
fn fixed_item_marker_is_repeated_with_its_owner_on_every_page() {
    let document = lay_out(
        "<ol style='position:fixed;left:0;top:0'><li>Header</li></ol><div style='height:220px'></div>",
        "",
    );
    assert!(document.page_count() >= 3);
    for page in document.pages() {
        let markers: Vec<_> = page
            .text_runs()
            .into_iter()
            .filter(|run| run.is_standalone_marker() && !run.text.trim().is_empty())
            .collect();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].text.trim_end(), "1.");
        assert!(
            page.paint_order_for_text_runs(&markers)
                .iter()
                .any(|event| matches!(event,PaintEvent::TextLine(line) if *line==markers[0].line))
        );
    }
}

#[test]
fn authored_marker_text_uses_its_pseudo_color_size_and_source() {
    let document = lay_out(
        "<ul><li>A</li></ul>",
        "li::marker{content:'XY';font-size:12px;color:red}",
    );
    let runs = document.page(0).unwrap().text_runs();
    let marker = runs.iter().find(|run| run.is_standalone_marker()).unwrap();
    assert_eq!(marker.text, "XY");
    assert_eq!(marker.font_size, 12.0);
    assert_eq!(marker.origin.0, 22.0);
    assert_eq!(
        (marker.color.r, marker.color.g, marker.color.b),
        (255, 0, 0)
    );
    assert_eq!(
        marker
            .glyphs
            .iter()
            .map(|glyph| glyph.text_range.clone())
            .collect::<Vec<_>>(),
        vec![0..1, 1..2]
    );
}

#[test]
fn marker_of_a_principal_box_wholly_above_the_page_is_not_projected() {
    let document = lay_out(
        "<ol><li>A</li></ol>",
        "li{position:absolute;top:-20px;height:10px}li::marker{font-size:40px;content:'X'}",
    );
    assert!(
        !document
            .pages()
            .flat_map(|page| page.text_runs())
            .any(|run| run.is_standalone_marker())
    );
}
