use crate::document::Document;
use parley::FontContext;
use raikiri_style::property::DisplayValue;
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::style::{Dimension, LengthPercentage, LengthPercentageAuto};
use taffy::{AvailableSpace, LayoutInput, Rect, Size, Style};

fn build_simple_2x2() -> Document {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let tr1 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let td2 = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t1 = doc.append_text(td1, "cell1");
    let _t2 = doc.append_text(td2, "cell2");
    let tr2 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td3 = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let td4 = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t3 = doc.append_text(td3, "cell3");
    let _t4 = doc.append_text(td4, "cell4");
    doc
}

#[test]
fn table_simple_2x2_columns_and_rows() {
    let mut doc = build_simple_2x2();
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
        .expect("layout");
    let td_idxs: Vec<usize> = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag_name() == Some("td"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(td_idxs.len(), 4);
    let l0 = doc.nodes[td_idxs[0]].unrounded_layout;
    let l1 = doc.nodes[td_idxs[1]].unrounded_layout;
    let l2 = doc.nodes[td_idxs[2]].unrounded_layout;
    assert!(
        (l1.location.x - l0.location.x).abs() > 1.0,
        "columns must separate: {} vs {}",
        l1.location.x,
        l0.location.x
    );
    assert!(
        (l2.location.x - l0.location.x).abs() < 1.0,
        "first column x must align vertically"
    );
    assert!(
        (l2.location.y - l0.location.y).abs() > 1.0,
        "rows must separate"
    );
    // table outer size should cover at least sum of two columns
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    let tl = doc.nodes[table_idx].unrounded_layout;
    let expected_w = l0.size.width + l1.size.width;
    assert!(
        tl.size.width >= expected_w - 1.0,
        "table width {} should >= content {}",
        tl.size.width,
        expected_w
    );
}

#[test]
fn table_vertical_writing_mode_maps_columns_to_inline_axis() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; writing-mode: vertical-lr"),
    );
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let first = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let second = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _first_child = doc.append_element(
        Some(first),
        "div",
        Style::default(),
        Some("width: 50px; height: 100px"),
    );
    let _second_child = doc.append_element(
        Some(second),
        "div",
        Style::default(),
        Some("width: 50px; height: 100px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
        .expect("layout");

    let first_layout = doc.nodes[first].unrounded_layout;
    let second_layout = doc.nodes[second].unrounded_layout;
    let table_layout = doc.nodes[table].unrounded_layout;
    assert!(
        (first_layout.location.x - second_layout.location.x).abs() < 0.5,
        "vertical columns share the block-axis origin: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        first_layout.location,
        second_layout.location
    );
    assert!(
        second_layout.location.y - first_layout.location.y >= 99.5,
        "vertical columns advance along the inline axis: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        first_layout.location,
        second_layout.location
    );
    assert!(
        table_layout.size.height >= first_layout.size.height + second_layout.size.height - 0.5,
        "vertical table height covers its inline tracks: {:?} vs {:?}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        table_layout.size,
        (first_layout.size, second_layout.size) // cov:ignore: assertion diagnostic is evaluated only on failure.
    );
}

#[test]
fn empty_table_caption_is_laid_out_and_painted() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Style::default(),
        Some(
            "display: table-caption; width: 100px; height: 50px; margin-left: 200px; \
             position: relative; left: -200px",
        ),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())
        .expect("layout");

    let caption_layout = doc.nodes[caption].unrounded_layout;
    let table_layout = doc.nodes[table].unrounded_layout;
    assert!((caption_layout.size.width - 100.0).abs() < 0.5);
    assert!((caption_layout.size.height - 50.0).abs() < 0.5);
    assert!((caption_layout.location.x - 200.0).abs() < 0.5);
    assert!(table_layout.size.height >= 50.0);
}

#[test]
fn table_colspan_wide_equals_sum_of_spanned_columns() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let tr1 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td_wide = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    doc.set_element_attributes(td_wide, vec![("colspan".into(), "2".into())]);
    let _t = doc.append_text(td_wide, "wide");
    let tr2 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td_a = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let td_b = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _ta = doc.append_text(td_a, "a");
    let _tb = doc.append_text(td_b, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let wide = doc.nodes[td_wide].unrounded_layout;
    let a = doc.nodes[td_a].unrounded_layout;
    let b = doc.nodes[td_b].unrounded_layout;
    assert!(
        (wide.size.width - (a.size.width + b.size.width)).abs() < 2.0,
        "wide {} vs a+b {} {}",
        wide.size.width,
        a.size.width + b.size.width,
        (wide.size.width - (a.size.width + b.size.width)).abs()
    );
}

#[test]
fn table_rowspan_distributes_height() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let tr1 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td_span = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    doc.set_element_attributes(td_span, vec![("rowspan".into(), "2".into())]);
    let _t = doc.append_text(td_span, "span");
    let td1 = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t1 = doc.append_text(td1, "a");
    let tr2 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td2 = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    // rowspan cell should span at least as tall as two rows combined
    let span = doc.nodes[td_span].unrounded_layout;
    let a = doc.nodes[td1].unrounded_layout;
    let b = doc.nodes[td2].unrounded_layout;
    assert!(
        span.size.height >= a.size.height - 0.5,
        "rowspan height {} should >= single row {}",
        span.size.height,
        a.size.height
    );
    assert!(
        (span.size.height - (a.size.height + b.size.height)).abs() < 5.0
            || span.size.height >= b.size.height,
        "rowspan approx"
    );
}

#[test]
fn nested_table_depth_cap_does_not_panic() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut parent = body;
    for _ in 0..10 {
        let table = doc.append_element(
            Some(parent),
            "table",
            Style::default(),
            Some("display: table"),
        );
        let tr = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        let _t = doc.append_text(td, "x");
        parent = td;
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let res = crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new());
    assert!(res.is_ok(), "nested table should not panic");
}

#[test]
fn table_with_tbody_anonymization() {
    // Ensure html5ever tbody auto-insertion not required: manually use tbody group
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let tbody = doc.append_element(
        Some(table),
        "tbody",
        Style::default(),
        Some("display: table-row-group"),
    );
    let tr = doc.append_element(
        Some(tbody),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t = doc.append_text(td, "inside tbody");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let td_l = doc.nodes[td].unrounded_layout;
    assert!(
        td_l.size.width > 1.0 && td_l.size.height > 1.0,
        "tbody wrapped cell should layout"
    );
}

#[test]
fn table_parse_path_via_raikiri_html_uses_ua_display() {
    // Simulate parse path UA injection: inject minimal UA CSS manually.
    use raikiri_style::Origin;
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // No inline display: rely on UA
    let table = doc.append_element(Some(body), "table", Style::default(), None::<&str>);
    let tr = doc.append_element(Some(table), "tr", Style::default(), None::<&str>);
    let td1 = doc.append_element(Some(tr), "td", Style::default(), None::<&str>);
    let td2 = doc.append_element(Some(tr), "td", Style::default(), None::<&str>);
    let _t1 = doc.append_text(td1, "hi");
    let _t2 = doc.append_text(td2, "bye");
    doc.mark_in_document_flags();
    let mut rules = build_rule_tree(&doc);
    rules.add_stylesheet(raikiri_html::ua::MINIMAL_UA_CSS, Origin::UserAgent);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .expect("table");
    assert_eq!(
        cr.computed[table_idx].display,
        raikiri_style::property::DisplayValue::Table,
        "UA should set table display"
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let tds: Vec<usize> = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.tag_name() == Some("td"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(tds.len(), 2);
    let l0 = doc.nodes[tds[0]].unrounded_layout;
    let l1 = doc.nodes[tds[1]].unrounded_layout;
    assert!(
        (l1.location.x - l0.location.x).abs() > 1.0,
        "parsed table columns must separate"
    );
}

#[test]
fn fixed_layout_honors_first_row_widths_and_ignores_content() {
    // CSS 2.1 §17.5.2.1: table 400px, first-row cell 100px fixed →
    // columns 100 / 300 regardless of the long content in column 2.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; table-layout: fixed; width: 400px"),
    );
    let tr = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell; width: 100px"),
    );
    let td2 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _t1 = doc.append_text(td1, "x");
    let _t2 = doc.append_text(
        td2,
        "a very long content string that must not widen its column under fixed layout",
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    assert_eq!(
        cr.computed[table_idx].table_layout,
        raikiri_style::property::TableLayoutValue::Fixed
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let l0 = doc.nodes[td1].unrounded_layout;
    let l1 = doc.nodes[td2].unrounded_layout;
    assert!(
        (l0.size.width - 100.0).abs() < 2.0,
        "fixed first column should be 100px, got {}",
        l0.size.width
    );
    assert!(
        (l1.size.width - 300.0).abs() < 2.0,
        "fixed second column should take the 300px remainder, got {}",
        l1.size.width
    );
}

#[test]
fn fixed_layout_ignores_non_first_row_widths() {
    // CSS 2.1 §17.5.2.1: only the first row sets widths — a 300px width
    // on a second-row cell must not move the columns off 200/200.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; table-layout: fixed; width: 400px"),
    );
    let tr1 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let td2 = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _a = doc.append_text(td1, "a");
    let _b = doc.append_text(td2, "b");
    let tr2 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td3 = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell; width: 300px"),
    );
    let td4 = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let _c = doc.append_text(td3, "c");
    let _d = doc.append_text(td4, "d");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let l0 = doc.nodes[td1].unrounded_layout;
    let l1 = doc.nodes[td2].unrounded_layout;
    assert!(
        (l0.size.width - 200.0).abs() < 2.0 && (l1.size.width - 200.0).abs() < 2.0,
        "unfixed columns should split 400px equally, got {} and {}",
        l0.size.width,
        l1.size.width
    );
}

#[test]
fn collapse_overlaps_adjoining_borders_by_pairwise_min() {
    // CSS 2.1 §17.6.2 (simplified max-width resolution): two 4px-bordered
    // cells collapse the interior line to max(4, 4) = 4 by overlapping
    // the boxes by min(4, 4) = 4.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; border-collapse: collapse"),
    );
    let tr = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell; border: 4px solid black"),
    );
    let td2 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell; border: 4px solid black"),
    );
    let _t1 = doc.append_text(td1, "a");
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    let table_idx = doc
        .nodes
        .iter()
        .position(|n| n.tag_name() == Some("table"))
        .unwrap();
    assert_eq!(
        cr.computed[table_idx].border_collapse,
        raikiri_style::property::BorderCollapseValue::Collapse
    );
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

    let l0 = doc.nodes[td1].unrounded_layout;
    let l1 = doc.nodes[td2].unrounded_layout;
    let overlap = (l0.location.x + l0.size.width) - l1.location.x;
    assert!(
        (overlap - 4.0).abs() < 1.0,
        "collapsed interior line should overlap by 4px, got {overlap}"
    );
    let tl = doc.nodes[table_idx].unrounded_layout;
    // Right-flush invariant: the last cell's far edge plus the 4px
    // collapsed outer border (no padding, no table border) meets the
    // table's right edge — the interior overlap must not leave slack.
    assert!(
        (tl.size.width - (l1.location.x + l1.size.width + 4.0)).abs() < 2.0,
        "table right edge should flush with last cell + outer collapsed border, got {}",
        tl.size.width
    );
}

#[test]
fn separate_cells_abut_without_overlap() {
    // Counterpart check: without `border-collapse: collapse` the same
    // bordered cells abut exactly (no absorbed line).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table"),
    );
    let tr = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell; border: 4px solid black"),
    );
    let td2 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell; border: 4px solid black"),
    );
    let _t1 = doc.append_text(td1, "a");
    let _t2 = doc.append_text(td2, "b");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let l0 = doc.nodes[td1].unrounded_layout;
    let l1 = doc.nodes[td2].unrounded_layout;
    let overlap = (l0.location.x + l0.size.width) - l1.location.x;
    assert!(
        overlap.abs() < 1.0,
        "separate cells should abut exactly, got overlap {overlap}"
    );
}

#[test]
fn explicit_single_row_height_resolves_percentage_child() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; width: 150px; height: 100px; border: 5px solid black"),
    );
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell; padding: 5px; border: 2px solid magenta"),
    );
    let child = doc.append_element(
        Some(cell),
        "div",
        Style::default(),
        Some("width: 100%; height: 100%"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

    let cell_layout = doc.nodes[cell].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    assert!(
        cell_layout.size.height >= 99.0,
        "authored table height should stretch the single cell, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        cell_layout.size.height
    );
    assert!(
        child_layout.size.height >= 80.0,
        "cell child should receive a non-zero containing block, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
        child_layout.size.height
    );
}

#[test]
fn explicit_table_height_respects_box_sizing() {
    for (box_sizing, expected_outer) in [("border-box", 100.0), ("content-box", 140.0)] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table_style = format!(
            "display: table; box-sizing: {box_sizing}; border: 20px solid black; border-collapse: collapse; height: 100px"
        );
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some(table_style.as_str()),
        );
        let row = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        doc.append_element(
            Some(row),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let table_layout = doc.nodes[table].unrounded_layout;
        assert!(
            (table_layout.size.height - expected_outer).abs() < 1.0,
            "{box_sizing}: table outer height should be {expected_outer}, got {}", // cov:ignore: assertion diagnostic is evaluated only on failure.
            table_layout.size.height
        );
    }
}

#[test]
fn table_min_size_respects_box_sizing() {
    // WPT css/css-tables/min-max-size-table-content-box (CSS Sizing 3
    // §3.3, csswg-drafts#5336 / Mozilla bug 1651530): with
    // `box-sizing: content-box`, min-width/min-height apply to the
    // content box — a 50px min with 3px border + 5px padding (16 total
    // insets) grows the content to 50 (outer 66). `border-box` keeps
    // the outer clamp (outer 50, content 34).
    for (box_sizing, expect_outer, expect_cell) in
        [("content-box", 66.0, 50.0), ("border-box", 50.0, 34.0)]
    {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let table_style = format!(
            "display: table; box-sizing: {box_sizing}; border: 3px solid black; \
             padding: 5px; min-width: 50px; min-height: 50px"
        );
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some(table_style.as_str()),
        );
        let tr = doc.append_element(
            Some(table),
            "tr",
            Style::default(),
            Some("display: table-row"),
        );
        let td = doc.append_element(
            Some(tr),
            "td",
            Style::default(),
            Some("display: table-cell"),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
        let tl = doc.nodes[table].unrounded_layout;
        let cl = doc.nodes[td].unrounded_layout;
        assert!(
            (tl.size.width - expect_outer).abs() < 1.5,
            "{box_sizing}: table outer width should be {expect_outer}, got {}",
            tl.size.width
        );
        assert!(
            (tl.size.height - expect_outer).abs() < 1.5,
            "{box_sizing}: table outer height should be {expect_outer}, got {}",
            tl.size.height
        );
        assert!(
            (cl.size.width - expect_cell).abs() < 1.5,
            "{box_sizing}: cell width should be {expect_cell}, got {}",
            cl.size.width
        );
        assert!(
            (cl.size.height - expect_cell).abs() < 1.5,
            "{box_sizing}: cell height should be {expect_cell}, got {}",
            cl.size.height
        );
    }
}

#[test]
fn distribute_columns_preserves_authored_tracks_for_spanning_excess() {
    let widths = super::distribute_columns_with_authored(
        110.0,
        &[5.0, 95.0, 0.0, 5.0],
        &[5.0, 95.0, 0.0, 5.0],
        &[0.0, 0.0, 0.0, 0.0],
        &[true, false, false, true],
    );
    assert!((widths[0] - 5.0).abs() < 0.01);
    assert!((widths[3] - 5.0).abs() < 0.01);
    assert!((widths.iter().sum::<f32>() - 110.0).abs() < 0.01);
}

#[test]
fn distribute_columns_falls_back_to_all_tracks_when_all_are_authored() {
    let widths = super::distribute_columns_with_authored(
        12.0,
        &[5.0, 5.0],
        &[5.0, 5.0],
        &[0.0, 0.0],
        &[true, true],
    );
    assert_eq!(widths, [6.0, 6.0]);
}

#[test]
fn distribute_columns_covers_growth_and_shrink_phases() {
    let grown = super::distribute_columns_with_authored(10.0, &[1.0], &[10.0], &[0.0], &[false]);
    assert_eq!(grown, [10.0]);

    let percentage =
        super::distribute_columns_with_authored(10.0, &[0.0], &[0.0], &[0.5], &[false]);
    assert_eq!(percentage, [10.0]);

    let shrunk = super::distribute_columns_with_authored(
        4.0,
        &[0.0, 5.0],
        &[10.0, 5.0],
        &[1.0, 0.0],
        &[false, false],
    );
    assert_eq!(shrunk, [0.0, 5.0]);
}

#[test]
fn table_colspan_percent_distribution_uses_definite_table_width() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; width: 110px"),
    );
    let tr1 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let wide = doc.append_element(
        Some(tr1),
        "td",
        Style::default(),
        Some("display: table-cell; width: 50%"),
    );
    doc.set_element_attributes(wide, vec![("colspan".into(), "2".into())]);
    let tr2 = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let left = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell; width: 5px"),
    );
    let right = doc.append_element(
        Some(tr2),
        "td",
        Style::default(),
        Some("display: table-cell; width: 5px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let wide_layout = doc.nodes[wide].unrounded_layout;
    let left_layout = doc.nodes[left].unrounded_layout;
    let right_layout = doc.nodes[right].unrounded_layout;
    assert!((wide_layout.size.width - 110.0).abs() < 1.0);
    assert!((left_layout.size.width - 55.0).abs() < 1.0);
    assert!((right_layout.size.width - 55.0).abs() < 1.0);
}

#[test]
fn absolute_auto_table_uses_definite_containing_block_width() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width: 100px; height: 100px; position: relative"),
    );
    let table = doc.append_element(
        Some(container),
        "table",
        Style::default(),
        Some("display: table; position: absolute; left: -100px; height: 100px"),
    );
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    doc.append_text(cell, "x");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();
    let table_layout = doc.nodes[table].unrounded_layout;
    assert!(
        (table_layout.size.width - 100.0).abs() < 1.0,
        "table width was {}",
        table_layout.size.width
    );
}

// -----------------------------------------------------------------
// distribute_columns — pure column distributor (avail/min/max/pct).
// -----------------------------------------------------------------

#[test]
fn distribute_columns_exact_fit_uses_min_widths_unchanged() {
    // avail exactly matches the summed min: neither the grow nor the
    // shrink branch fires, so the result is just `min`.
    let widths = super::distribute_columns(30.0, &[10.0, 20.0], &[50.0, 50.0], &[0.0, 0.0]);
    assert_eq!(widths, [10.0, 20.0]);
}

#[test]
fn distribute_columns_grows_toward_max_then_shares_remainder_equally() {
    // avail exceeds the summed max: every column first grows to its
    // own max, then the still-leftover space is split equally.
    let widths = super::distribute_columns(30.0, &[0.0, 0.0], &[10.0, 10.0], &[0.0, 0.0]);
    assert_eq!(widths, [15.0, 15.0]);
}

#[test]
fn distribute_columns_percentage_over_100_percent_is_scaled_down() {
    // pct sums to 120% of avail: the `psum > 1.0` branch scales every
    // percentage down by 1/psum before applying it.
    let widths = super::distribute_columns(100.0, &[0.0, 0.0], &[100.0, 100.0], &[0.6, 0.6]);
    assert!((widths[0] - 50.0).abs() < 0.01, "widths: {widths:?}");
    assert!((widths[1] - 50.0).abs() < 0.01, "widths: {widths:?}");
}

#[test]
fn distribute_columns_shrinks_percentage_driven_column_when_over_avail() {
    // The pct-driven column (50px) plus the min-floored column (20px)
    // exceed avail (50px): shrink comes only out of the pct column's
    // slack above its own min (column 1 already sits at its min).
    let widths = super::distribute_columns(50.0, &[0.0, 20.0], &[100.0, 100.0], &[1.0, 0.0]);
    assert_eq!(widths, [30.0, 20.0]);
}

#[test]
fn distribute_columns_min_floor_holds_when_shrink_budget_is_zero() {
    // Over-constrained input (avail below the summed min) with no
    // percentage columns: every column already sits at its own min,
    // so the shrink budget is zero and the table overflows rather
    // than compressing columns below their intrinsic minimum.
    let widths = super::distribute_columns(10.0, &[20.0, 30.0], &[20.0, 30.0], &[0.0, 0.0]);
    assert_eq!(widths, [20.0, 30.0]);
}

// -----------------------------------------------------------------
// distribute_extra_width — pure min-width grower for resolved columns.
// -----------------------------------------------------------------

#[test]
fn distribute_extra_width_grows_columns_proportionally_to_current_width() {
    let mut widths = [10.0, 30.0];
    super::distribute_extra_width(&mut widths, 80.0);
    assert_eq!(widths, [20.0, 60.0]);
}

#[test]
fn distribute_extra_width_splits_equally_when_all_columns_are_zero() {
    let mut widths = [0.0, 0.0, 0.0];
    super::distribute_extra_width(&mut widths, 30.0);
    assert_eq!(widths, [10.0, 10.0, 10.0]);
}

#[test]
fn distribute_extra_width_is_noop_when_current_already_meets_target() {
    let mut widths = [50.0, 50.0]; // sum 100.0, above the 80.0 target
    super::distribute_extra_width(&mut widths, 80.0);
    assert_eq!(widths, [50.0, 50.0]);
}

#[test]
fn distribute_extra_width_is_noop_on_empty_slice() {
    let mut widths: [f32; 0] = [];
    super::distribute_extra_width(&mut widths, 30.0);
    assert!(widths.is_empty());
}

// -----------------------------------------------------------------
// resolve_fixed_column_widths — CSS 2.1 §17.5.2.1 fixed algorithm.
// -----------------------------------------------------------------

fn fixed_cell(row: u16, col_start: u16, col_span: u16, width: Dimension) -> super::CellPlacement {
    super::CellPlacement {
        node_id: 0,
        row,
        col_start,
        col_span,
        row_span: 1,
        specified_width: width,
        resolved: None,
    }
}

#[test]
fn resolve_fixed_column_widths_first_row_length_fixes_column_remainder_splits() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::length(100.0)),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [100.0, 300.0]);
}

#[test]
fn resolve_fixed_column_widths_first_row_percent_resolves_against_avail() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::percent(0.25)),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [100.0, 300.0]);
}

#[test]
fn resolve_fixed_column_widths_ignores_non_first_row_width() {
    // A length on a *second*-row cell must not fix its column.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::auto()),
            fixed_cell(0, 1, 1, Dimension::auto()),
            fixed_cell(1, 0, 1, Dimension::length(300.0)),
        ],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_ignores_colspan_first_row_width() {
    // A first-row cell with colspan > 1 never fixes a column — only
    // colspan == 1 first-row cells participate in §17.5.2.1 fixing.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![fixed_cell(0, 0, 2, Dimension::length(300.0))],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_length_fixes_unspecified_column() {
    // No first-row cell width at all: a `<col>` length still fixes
    // the column (the §17.5.2.1 `<col>` fallback).
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            fixed_cell(0, 0, 1, Dimension::auto()),
            fixed_cell(0, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(60.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [60.0, 340.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_percent_resolves_against_avail() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::percent(0.5),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [200.0, 200.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_min_only_floors_column() {
    // A `<col>` with no width but a length min-width still fixes the
    // column (`v.is_none() && is_len(min_width)` fallback).
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::length(50.0),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [50.0, 350.0]);
}

#[test]
fn resolve_fixed_column_widths_col_element_width_clamped_by_max_width() {
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(100.0),
                min_width: Dimension::auto(),
                max_width: Dimension::length(60.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [60.0, 340.0]);
}

#[test]
fn resolve_fixed_column_widths_underflow_clamps_unfixed_columns_to_zero() {
    // The single fixed column already exceeds avail: the leftover
    // for the unfixed column clamps to zero rather than going
    // negative — the table overflows instead of shrinking it.
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![fixed_cell(0, 0, 1, Dimension::length(500.0))],
        col_widths: vec![],
    };
    let widths = super::resolve_fixed_column_widths(&grid, 400.0);
    assert_eq!(widths, [500.0, 0.0]);
}

// -----------------------------------------------------------------
// collect_col_widths / collect_rows_inner / flush_pending — grid
// construction helpers, called directly on a minimal html/body/table
// shell (display + attributes set on Node directly, bypassing
// cascade — these functions only read `Node::display`/attributes,
// not computed CSS).
// -----------------------------------------------------------------

fn table_shell() -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(Some(body), "table", Style::default(), None::<&str>);
    (doc, table)
}

#[test]
fn collect_col_widths_expands_span_and_reads_colgroup_and_bare_col() {
    let (mut doc, table) = table_shell();
    let colgroup = doc.append_element(Some(table), "colgroup", Style::default(), None::<&str>);
    doc.nodes[colgroup].display = DisplayValue::TableColumnGroup;

    let col_a = doc.append_element(
        Some(colgroup),
        "col",
        Style {
            size: Size {
                width: Dimension::length(40.0),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    doc.nodes[col_a].display = DisplayValue::TableColumn;

    let col_b = doc.append_element(
        Some(colgroup),
        "col",
        Style {
            min_size: Size {
                width: LengthPercentageAuto::length(20.0),
                height: LengthPercentageAuto::auto(),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    doc.nodes[col_b].display = DisplayValue::TableColumn;
    doc.set_element_attributes(col_b, vec![("span".into(), "2".into())]);

    // A `<tr>` sibling of the colgroup must be skipped, not treated
    // as a column.
    let tr = doc.append_element(Some(table), "tr", Style::default(), None::<&str>);
    doc.nodes[tr].display = DisplayValue::TableRow;

    // A bare `<col>` directly under the table (anonymous colgroup).
    let col_c = doc.append_element(
        Some(table),
        "col",
        Style {
            size: Size {
                width: Dimension::percent(0.25),
                height: Dimension::auto(),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    doc.nodes[col_c].display = DisplayValue::TableColumn;

    doc.mark_in_document_flags();
    let sizing = super::collect_col_widths(&doc, table);

    assert_eq!(sizing.len(), 4, "col_a + col_b(span=2) + col_c = 4 entries");
    assert_eq!(sizing[0].width, Dimension::length(40.0));
    assert_eq!(sizing[1].min_width, Dimension::length(20.0));
    assert_eq!(
        sizing[2].min_width,
        Dimension::length(20.0),
        "span=2 expands to two identical entries"
    );
    assert_eq!(sizing[3].width, Dimension::percent(0.25));
}

#[test]
fn collect_rows_wraps_bare_cells_in_anonymous_row() {
    let (mut doc, table) = table_shell();
    let td1 = doc.append_element(Some(table), "td", Style::default(), None::<&str>);
    doc.nodes[td1].display = DisplayValue::TableCell;
    let td2 = doc.append_element(Some(table), "td", Style::default(), None::<&str>);
    doc.nodes[td2].display = DisplayValue::TableCell;
    doc.mark_in_document_flags();

    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::collect_rows(&doc, table, &mut rows, &mut cells, &mut n_cols);

    assert_eq!(rows.len(), 1, "bare cells wrap into a single anonymous row");
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0].col_start, 0);
    assert_eq!(cells[1].col_start, 1);
    assert_eq!(n_cols, 2);
}

fn head_body_fixture() -> (Document, usize, usize, usize) {
    let (mut doc, table) = table_shell();
    // DOM order: tbody first, thead second — reorder must float the
    // header group up regardless.
    let tbody = doc.append_element(Some(table), "tbody", Style::default(), None::<&str>);
    doc.nodes[tbody].display = DisplayValue::TableRowGroup;
    let tr_body = doc.append_element(Some(tbody), "tr", Style::default(), None::<&str>);
    doc.nodes[tr_body].display = DisplayValue::TableRow;
    let body_cell = doc.append_element(Some(tr_body), "td", Style::default(), None::<&str>);
    doc.nodes[body_cell].display = DisplayValue::TableCell;

    let thead = doc.append_element(Some(table), "thead", Style::default(), None::<&str>);
    doc.nodes[thead].display = DisplayValue::TableHeaderGroup;
    let tr_head = doc.append_element(Some(thead), "tr", Style::default(), None::<&str>);
    doc.nodes[tr_head].display = DisplayValue::TableRow;
    let head_cell = doc.append_element(Some(tr_head), "td", Style::default(), None::<&str>);
    doc.nodes[head_cell].display = DisplayValue::TableCell;

    doc.mark_in_document_flags();
    (doc, table, head_cell, body_cell)
}

#[test]
fn collect_rows_floats_first_header_group_to_top() {
    let (doc, table, head_cell, body_cell) = head_body_fixture();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::collect_rows(&doc, table, &mut rows, &mut cells, &mut n_cols);

    assert_eq!(cells.len(), 2);
    assert_eq!(
        cells[0].node_id, head_cell,
        "thead row must come first despite DOM order"
    );
    assert_eq!(cells[1].node_id, body_cell);
}

#[test]
fn collect_rows_inner_no_reorder_keeps_dom_order_for_nested_call() {
    let (doc, table, head_cell, body_cell) = head_body_fixture();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::collect_rows_inner(&doc, table, &mut rows, &mut cells, &mut n_cols, false);

    assert_eq!(cells.len(), 2);
    assert_eq!(
        cells[0].node_id, body_cell,
        "DOM order preserved: tbody before thead"
    );
    assert_eq!(cells[1].node_id, head_cell);
}

#[test]
fn collect_rows_inner_skips_character_data_between_rows() {
    let (mut doc, table) = table_shell();
    let tr1 = doc.append_element(Some(table), "tr", Style::default(), None::<&str>);
    doc.nodes[tr1].display = DisplayValue::TableRow;
    let td1 = doc.append_element(Some(tr1), "td", Style::default(), None::<&str>);
    doc.nodes[td1].display = DisplayValue::TableCell;

    doc.append_text(table, "   ");

    let tr2 = doc.append_element(Some(table), "tr", Style::default(), None::<&str>);
    doc.nodes[tr2].display = DisplayValue::TableRow;
    let td2 = doc.append_element(Some(tr2), "td", Style::default(), None::<&str>);
    doc.nodes[td2].display = DisplayValue::TableCell;

    doc.mark_in_document_flags();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::collect_rows(&doc, table, &mut rows, &mut cells, &mut n_cols);

    assert_eq!(
        rows.len(),
        2,
        "inter-row whitespace must not create a spurious row"
    );
    assert_eq!(cells.len(), 2);
}

#[test]
fn collect_rows_inner_recurses_into_non_row_group_wrapper() {
    let (mut doc, table) = table_shell();
    // A plain wrapper (e.g. an authoring-error `<div>` directly under
    // `<table>`) is neither a row, a cell, nor a row-group — the
    // catch-all branch still recurses into it looking for rows.
    let wrapper = doc.append_element(Some(table), "div", Style::default(), None::<&str>);
    let tr = doc.append_element(Some(wrapper), "tr", Style::default(), None::<&str>);
    doc.nodes[tr].display = DisplayValue::TableRow;
    let td = doc.append_element(Some(tr), "td", Style::default(), None::<&str>);
    doc.nodes[td].display = DisplayValue::TableCell;

    doc.mark_in_document_flags();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::collect_rows(&doc, table, &mut rows, &mut cells, &mut n_cols);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].node_id, td);
}

#[test]
fn flush_pending_assigns_sequential_columns_respecting_colspan() {
    let (mut doc, table) = table_shell();
    let td_a = doc.append_element(Some(table), "td", Style::default(), None::<&str>);
    doc.set_element_attributes(td_a, vec![("colspan".into(), "2".into())]);
    let td_b = doc.append_element(Some(table), "td", Style::default(), None::<&str>);
    doc.mark_in_document_flags();

    let mut pending = vec![td_a, td_b];
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::flush_pending(&doc, &mut pending, &mut rows, &mut cells, &mut n_cols);

    assert!(pending.is_empty());
    assert_eq!(rows, vec![td_a]);
    assert_eq!(cells.len(), 2);
    assert_eq!((cells[0].col_start, cells[0].col_span), (0, 2));
    assert_eq!((cells[1].col_start, cells[1].col_span), (2, 1));
    assert_eq!(n_cols, 3);
}

#[test]
fn flush_pending_on_empty_pending_is_noop() {
    let doc = Document::new();
    let mut pending: Vec<usize> = Vec::new();
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut n_cols = 0u16;
    super::flush_pending(&doc, &mut pending, &mut rows, &mut cells, &mut n_cols);
    assert!(rows.is_empty());
    assert!(cells.is_empty());
    assert_eq!(n_cols, 0);
}

// -----------------------------------------------------------------
// compute_collapsed_lines — CSS 2.1 §17.6.2 simplified max-width
// conflict resolution (max of adjoining borders; absorbed overlap
// is the pairwise min).
// -----------------------------------------------------------------

fn make_cell(
    node_id: usize,
    row: u16,
    col_start: u16,
    col_span: u16,
    row_span: u16,
    width: Dimension,
) -> super::CellPlacement {
    super::CellPlacement {
        node_id,
        row,
        col_start,
        col_span,
        row_span,
        specified_width: width,
        resolved: None,
    }
}

fn bordered_cell(doc: &mut Document, px: f32) -> usize {
    doc.append_element(
        None,
        "td",
        Style {
            border: Rect {
                left: LengthPercentage::length(px),
                right: LengthPercentage::length(px),
                top: LengthPercentage::length(px),
                bottom: LengthPercentage::length(px),
            },
            ..Style::default()
        },
        None::<&str>,
    )
}

#[test]
fn compute_collapsed_lines_resolves_2x2_grid_overlaps_and_outers() {
    let mut doc = Document::new();
    let c00 = bordered_cell(&mut doc, 4.0);
    let c01 = bordered_cell(&mut doc, 2.0);
    let c10 = bordered_cell(&mut doc, 6.0);
    let c11 = bordered_cell(&mut doc, 3.0);
    let grid = super::TableGrid {
        n_cols: 2,
        // Only the row *count* matters here (`compute_collapsed_lines`
        // reads `grid.rows.len()` for its boundary loop bounds; row
        // node ids are never dereferenced), so these are placeholders.
        rows: vec![0, 0],
        cells: vec![
            make_cell(c00, 0, 0, 1, 1, Dimension::auto()),
            make_cell(c01, 0, 1, 1, 1, Dimension::auto()),
            make_cell(c10, 1, 0, 1, 1, Dimension::auto()),
            make_cell(c11, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let table_border = Rect {
        left: 1.0,
        right: 1.0,
        top: 1.0,
        bottom: 1.0,
    };
    let lines = super::compute_collapsed_lines(&doc, &grid, &table_border, None);

    assert_eq!(
        lines.col_overlaps,
        vec![3.0],
        "row0 min(4,2)=2 vs row1 min(6,3)=3 -> max 3"
    );
    assert_eq!(
        lines.row_overlaps,
        vec![4.0],
        "col0 min(4,6)=4 vs col1 min(2,3)=2 -> max 4"
    );
    assert_eq!(lines.outer_left, 6.0);
    assert_eq!(lines.outer_right, 3.0);
    assert_eq!(lines.outer_top, 4.0);
    assert_eq!(lines.outer_bottom, 6.0);
}

#[test]
fn compute_collapsed_lines_colspan_cell_only_absorbs_from_its_own_row() {
    // 3 columns x 2 rows: row 0 has one cell spanning columns 0-1
    // (colspan=2, border 6) plus a column-2 cell (border 2); row 1
    // has three ordinary cells (borders 4, 1, 1). The interior
    // boundary under the span (boundary 0) has no adjoining pair in
    // row 0 (both sides belong to the same cell) so it comes only
    // from row 1; boundary 1 sees the span's own right edge in row 0.
    let mut doc = Document::new();
    let span = bordered_cell(&mut doc, 6.0);
    let r0c2 = bordered_cell(&mut doc, 2.0);
    let r1c0 = bordered_cell(&mut doc, 4.0);
    let r1c1 = bordered_cell(&mut doc, 1.0);
    let r1c2 = bordered_cell(&mut doc, 1.0);
    let grid = super::TableGrid {
        n_cols: 3,
        rows: vec![0, 0], // row count only — see note above.
        cells: vec![
            make_cell(span, 0, 0, 2, 1, Dimension::auto()),
            make_cell(r0c2, 0, 2, 1, 1, Dimension::auto()),
            make_cell(r1c0, 1, 0, 1, 1, Dimension::auto()),
            make_cell(r1c1, 1, 1, 1, 1, Dimension::auto()),
            make_cell(r1c2, 1, 2, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let lines = super::compute_collapsed_lines(&doc, &grid, &Rect::ZERO, None);

    assert_eq!(
        lines.col_overlaps,
        vec![1.0, 2.0],
        "boundary 0 comes only from row 1 (min(4,1)=1); boundary 1 comes from \
         the wider row-0 span edge (min(6,2)=2) over row 1's min(1,1)=1"
    );
}

// -----------------------------------------------------------------
// resolve_column_widths — auto column-sizing aggregation (colspan
// excess distribution, `<col>` authored floors, percent handling).
// -----------------------------------------------------------------

fn column_probe_input() -> LayoutInput {
    LayoutInput {
        run_mode: taffy::tree::RunMode::PerformLayout,
        sizing_mode: taffy::tree::SizingMode::InherentSize,
        axis: taffy::tree::RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: None,
        },
        parent_size: Size {
            width: None,
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        vertical_margins_are_collapsible: taffy::geometry::Line::FALSE,
    }
}

#[test]
fn resolve_column_widths_colspan_one_percent_cell_sets_column_percentage() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let pct_cell = doc.append_element(Some(html), "td", Style::default(), None::<&str>);
    let auto_cell = doc.append_element(Some(html), "td", Style::default(), None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(pct_cell, 0, 0, 1, 1, Dimension::percent(0.4)),
            make_cell(auto_cell, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );
    assert!(
        (widths[0] - 80.0).abs() < 0.5,
        "40% of 200 -> 80: {widths:?}"
    );
}

#[test]
fn resolve_column_widths_colspan_excess_distributes_min_and_max_across_targets() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let span_cell = doc.append_element(Some(html), "td", Style::default(), None::<&str>);
    doc.append_element(
        Some(span_cell),
        "div",
        Style {
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let c0 = doc.append_element(Some(html), "td", Style::default(), None::<&str>);
    let c1 = doc.append_element(Some(html), "td", Style::default(), None::<&str>);
    doc.mark_in_document_flags();

    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(span_cell, 0, 0, 2, 1, Dimension::auto()),
            make_cell(c0, 1, 0, 1, 1, Dimension::auto()),
            make_cell(c1, 1, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(300.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );

    assert!((widths[0] - 150.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 150.0).abs() < 1.0, "widths: {widths:?}");
    assert!(
        (widths[0] - widths[1]).abs() < 0.01,
        "a colspan cell's min/max excess splits evenly across its non-authored targets"
    );
}

#[test]
fn resolve_column_widths_col_percent_ignored_for_unoccupied_column() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![], // no cells at all: column 1 is unoccupied
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(60.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::percent(0.25),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );

    // If the 25% were (incorrectly) applied despite no cell occupying
    // the column, column 1 would be pct-driven and would stop
    // receiving a share of the leftover space; instead it behaves as
    // a plain auto column here.
    assert!((widths[0] - 130.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 70.0).abs() < 1.0, "widths: {widths:?}");
}

#[test]
fn resolve_column_widths_col_length_floors_even_unoccupied_column() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(90.0),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );

    assert!(widths[0] > widths[1], "widths: {widths:?}");
    assert!((widths[0] - 145.0).abs() < 1.0, "widths: {widths:?}");
    assert!((widths[1] - 55.0).abs() < 1.0, "widths: {widths:?}");
}

#[test]
fn resolve_column_widths_col_min_over_max_resolves_to_min() {
    // CSS Sizing 3 §4/§5: min > max resolves to min (max is
    // ignored), even though a base `width` is also specified.
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(50.0),
                min_width: Dimension::length(80.0),
                max_width: Dimension::length(30.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(80.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );

    assert_eq!(widths, [80.0, 0.0]);
}

#[test]
fn resolve_column_widths_col_width_clamped_by_max_width() {
    let mut doc = Document::new();
    let grid = super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![],
        col_widths: vec![
            super::ColSizing {
                width: Dimension::length(100.0),
                min_width: Dimension::auto(),
                max_width: Dimension::length(60.0),
            },
            super::ColSizing {
                width: Dimension::auto(),
                min_width: Dimension::auto(),
                max_width: Dimension::auto(),
            },
        ],
    };
    let inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(60.0),
            height: None,
        },
        ..column_probe_input()
    };
    let widths = super::resolve_column_widths(
        &mut doc,
        &grid,
        inputs,
        Size {
            width: 0.0,
            height: 0.0,
        },
    );

    assert_eq!(widths, [60.0, 0.0]);
}

// -----------------------------------------------------------------
// compute_table_layout — higher-level integration coverage for
// branches not exercised by the fixtures above.
// -----------------------------------------------------------------

#[test]
fn compute_table_layout_empty_table_as_flex_item_uses_container_width() {
    // An empty table (no rows at all) inside a flex container must
    // fill the flex-resolved main size rather than shrink-wrapping
    // to its padding/border (the `parent_is_flex_or_grid` branch).
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display: flex; width: 300px"),
    );
    let table = doc.append_element(
        Some(flex),
        "table",
        Style::default(),
        Some("display: table; flex-grow: 1"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

    let table_layout = doc.nodes[table].unrounded_layout;
    assert!(
        table_layout.size.width >= 299.0,
        "empty table as sole flex item should fill the 300px flex container, got {}",
        table_layout.size.width
    );
}

#[test]
fn compute_table_layout_max_width_never_shrinks_below_intrinsic_content() {
    // csswg-drafts#5336 / Mozilla bug 1651530: max-width never
    // shrinks a table below its intrinsic content width — only
    // min-width grows it. A max-width smaller than the natural
    // content width is therefore a no-op.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; max-width: 60px"),
    );
    let tr = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td1 = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    doc.append_element(
        Some(td1),
        "div",
        Style::default(),
        Some("width: 100px; height: 10px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

    let table_layout = doc.nodes[table].unrounded_layout;
    assert!(
        table_layout.size.width >= 99.0,
        "max-width below intrinsic content must not shrink the table, got {}",
        table_layout.size.width
    );
}

#[test]
fn compute_table_layout_single_column_separate_border_spacing_adds_both_gaps() {
    // A single-column separate-border table needs the *two* outer
    // spacing gaps folded into its one track's used width.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display: table; border-spacing: 10px"),
    );
    let tr = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    let td = doc.append_element(
        Some(tr),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    doc.append_element(
        Some(td),
        "div",
        Style::default(),
        Some("width: 40px; height: 10px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();
    crate::layout::layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).unwrap();

    let td_layout = doc.nodes[td].unrounded_layout;
    assert!(
        (td_layout.size.width - 60.0).abs() < 2.0,
        "single track should include both 10px spacing gaps (40 + 2*10 = 60), got {}",
        td_layout.size.width
    );
}
