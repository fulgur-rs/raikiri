//! External-consumer coverage for the paint order a painter reads.
//!
//! Every type below is named through `raikiri_html` only, the way a
//! downstream painter that does not depend on the DOM crate would write it.

use raikiri_html::{
    ClipKind, DocumentLayout, LayoutOptions, LayoutStatus, PaintEvent, RenderResources,
    WarningKind, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, style: &str) -> DocumentLayout {
    let html = format!("<!doctype html><style>{style}</style><body>{body}</body>");
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    result
}

#[test]
fn absolute_box_paints_after_preceding_text() {
    let result = lay_out(
        "<p id=p>text</p><div id=a style='position:absolute;top:0;left:0;\
         width:50px;height:50px;background:red'></div>",
        "",
    );
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let events = page.paint_order();
    let pos = |pred: &dyn Fn(&PaintEvent<'_>) -> bool| events.iter().position(pred).unwrap();
    let text = pos(&|e| {
        matches!(e, PaintEvent::Text(f)
            if dom.parent(f.node()).and_then(|n| dom.attr(n, "id")) == Some("p"))
    });
    let abs = pos(&|e| matches!(e, PaintEvent::Box(f) if dom.attr(f.node(), "id") == Some("a")));
    assert!(text < abs);
}

#[test]
fn paint_order_fragments_belong_to_page() {
    let result = lay_out(
        "<div style='opacity:.5'><p style='height:150px'>a</p></div><img style='width:10px;height:10px'>\
         <div style='position:fixed;top:0'>header</div><p>b</p>",
        "@page { size: 300px 100px; margin: 0 }",
    );
    let mut checked = 0;
    for page in result.pages() {
        let all: Vec<_> = page
            .fragments()
            .map(|f| (f.node(), f.kind(), f.fragment_index()))
            .collect();
        for event in page.paint_order() {
            if let PaintEvent::Box(f) | PaintEvent::Text(f) | PaintEvent::Replaced(f) = event {
                assert!(
                    all.contains(&(f.node(), f.kind(), f.fragment_index())),
                    "page {} event fragment not in fragments()",
                    page.index()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no event carries a fragment");
}

#[test]
fn spanning_overflow_box_clips_on_each_page() {
    let result = lay_out(
        "<div style='overflow:hidden'><p style='height:150px'>a</p></div>",
        "@page { size: 300px 100px; margin: 0 }",
    );
    for index in 0..2 {
        let events = result.page(index).unwrap().paint_order();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PaintEvent::PushClip(_, ClipKind::Overflow))),
            "page {index}"
        );
    }
}

#[test]
fn multicol_records_an_approximation_warning() {
    let result = lay_out("<div style='columns:2'><p>a</p><p>b</p></div>", "");
    let approximated: Vec<_> = result
        .warnings()
        .iter()
        .filter(|w| matches!(w.kind, WarningKind::PaintOrderApproximated))
        .collect();
    assert_eq!(approximated.len(), 1, "{approximated:?}");
    assert!(approximated[0].node_id.is_some());
}

fn lay_out_with_font(body: &str, style: &str) -> DocumentLayout {
    let fonts = raikiri_html::FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{size:200px 150px;margin:0}} body {{margin:0;font:10px/10px Ahem}} {style}</style><body>{body}</body>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap();
    let LayoutStatus::Completed(result) = status else {
        panic!("layout must complete")
    };
    result
}

#[test]
fn supplied_text_lines_include_generated_and_ellipsis_inside_opacity_groups() {
    use raikiri_html::RunSource;
    use std::collections::{HashMap, HashSet};
    let result = lay_out_with_font(
        "<section style='opacity:.5'><p id=generated>Body</p><p id=ellipsis>Long text overflowing</p><p id=wrapped>A<span style='color:red'>B</span>CDEF</p></section>",
        "p {margin:0} #generated::before {content:'Before'} #generated::after {content:'After'} #ellipsis {width:40px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis} #wrapped {width:20px;word-break:break-all}",
    );
    let page = result.page(0).unwrap();
    let runs = page.text_runs();
    assert!(
        runs.iter()
            .any(|run| matches!(run.source, RunSource::Generated(_, _)))
    );
    assert!(
        runs.iter()
            .any(|run| matches!(run.source, RunSource::Ellipsis(_)))
    );
    let expected: HashSet<_> = runs.iter().map(|run| run.line).collect();
    assert!(
        expected.len() >= 5,
        "fixture must contain multiple real lines"
    );
    let mut counts = HashMap::new();
    let mut opacity_depth = 0;
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushOpacity(alpha) => {
                assert_eq!(alpha, 0.5);
                opacity_depth += 1;
            }
            PaintEvent::PopOpacity => {
                assert!(opacity_depth > 0);
                opacity_depth -= 1;
            }
            PaintEvent::TextLine(line) => {
                assert_eq!(opacity_depth, 1);
                *counts.entry(line).or_insert(0) += 1;
            }
            PaintEvent::Text(_) => panic!("line stream must not repeat fragment text events"),
            _ => {}
        }
    }
    assert_eq!(opacity_depth, 0);
    assert_eq!(counts.keys().copied().collect::<HashSet<_>>(), expected);
    assert!(counts.values().all(|&count| count == 1));
    assert!(
        page.paint_order()
            .iter()
            .any(|event| matches!(event, PaintEvent::Text(_)))
    );
}

#[test]
fn anonymous_and_repeated_lines_have_complete_events_on_each_page() {
    use std::collections::HashSet;
    let result = lay_out_with_font(
        "<header>Fixed</header><div class=flex>Anon<span>Box</span></div><p>First</p><p>Second</p>",
        "@page {size:200px 40px} header {position:fixed;top:0;left:100px;opacity:.5} .flex {display:flex} p {margin:0;height:30px}",
    );
    assert!(result.pages().count() >= 2);
    for page in result.pages() {
        let runs = page.text_runs();
        assert!(!runs.is_empty());
        let expected: HashSet<_> = runs.iter().map(|run| run.line).collect();
        let lines: Vec<_> = page
            .paint_order_for_text_runs(&runs)
            .into_iter()
            .filter_map(|event| match event {
                PaintEvent::TextLine(line) => Some(line),
                PaintEvent::Text(_) => panic!("fragment text in supplied line stream"),
                _ => None,
            })
            .collect();
        assert_eq!(
            lines.len(),
            expected.len(),
            "each page line is emitted once"
        );
        assert_eq!(lines.into_iter().collect::<HashSet<_>>(), expected);
        assert!(
            page.paint_order_for_text_runs(&[])
                .iter()
                .all(|event| !matches!(event, PaintEvent::TextLine(_) | PaintEvent::Text(_)))
        );
    }
}

#[test]
fn supplied_runs_cannot_reorder_intrinsic_text_lines() {
    let result = lay_out_with_font(
        "<p>First<br>Second<br>Third</p>",
        "p {margin:0;line-height:0}",
    );
    let page = result.page(0).unwrap();
    let mut runs = page.text_runs();
    let expected: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::TextLine(line) => Some(line),
            _ => None,
        })
        .collect();
    assert!(expected.len() >= 3);
    runs.reverse();
    let actual: Vec<_> = page
        .paint_order_for_text_runs(&runs)
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::TextLine(line) => Some(line),
            _ => None,
        })
        .collect();
    assert_eq!(actual, expected);
}
