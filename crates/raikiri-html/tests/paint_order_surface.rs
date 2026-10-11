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
fn multiline_inline_boxes_do_not_advertise_one_content_rectangle() {
    let result = lay_out_with_font(
        "<p><span id=wrapped>ABCDEFGHIJKL</span></p>",
        "p{width:30px;word-break:break-all}span{padding:2px;border:1px solid black}",
    );
    let page = result.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("wrapped"))
        .unwrap();
    assert!(
        fragment.paint_rect().height > 20.0,
        "fixture must wrap into several pieces"
    );
    assert_eq!(fragment.content_rect(), None);
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

#[test]
fn inline_opacity_groups_paint_at_the_positioned_stack_level() {
    let result = lay_out_with_font(
        "<p id=p>A<i id=before></i><span id=group>B</span><i id=above></i>\
         <i id=after></i><i id=flow></i></p>",
        "p {margin:0} i {display:inline-block;width:5px;height:5px;background:red} \
         #group {opacity:.5;background:blue} #before, #after {position:relative} \
         #above {position:relative;z-index:1}",
    );
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let runs = page.text_runs();
    let mut order = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushOpacity(_) => order.push("push".to_string()),
            PaintEvent::PopOpacity => order.push("pop".into()),
            PaintEvent::Box(fragment) => {
                if let Some(name) = dom.attr(fragment.node(), "id") {
                    order.push(name.to_string());
                }
            }
            _ => {}
        }
    }
    // In-flow atomic inlines first, then the group in tree order with the
    // positioned boxes of z-index auto, then positive z-index.
    assert_eq!(
        order,
        [
            "p", "flow", "before", "push", "group", "pop", "after", "above"
        ]
    );
}

#[test]
fn inline_opacity_groups_list_their_content_after_the_paragraph() {
    use std::collections::HashMap;
    let result = lay_out_with_font(
        "<p id=p>Aa <span id=outer>Bb <b id=inner>Cc</b> <i id=atomic></i> Dd</span> Ee</p>",
        "p {margin:0;width:60px} #outer {opacity:.5;background:blue} \
         #inner {opacity:.25;background:green} \
         #atomic {display:inline-block;width:5px;height:5px;background:red}",
    );
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let id = |node| dom.attr(node, "id");
    let runs = page.text_runs();
    let group_of = |text: &str| {
        let run = runs
            .iter()
            .find(|run| run.text.contains(text))
            .unwrap_or_else(|| panic!("no run for {text}"));
        page.text_run_opacity_group(run).and_then(id)
    };
    assert_eq!(group_of("Aa"), None);
    assert_eq!(group_of("Bb"), Some("outer"));
    assert_eq!(group_of("Cc"), Some("inner"));
    assert_eq!(group_of("Dd"), Some("outer"));
    assert_eq!(group_of("Ee"), None);

    // Every run is drawn by exactly one line event, in its own group.
    let run_key = |run: &raikiri_html::PositionedGlyphRun<'_>| {
        (run.line, run.origin.0.to_bits(), run.origin.1.to_bits())
    };
    let mut drawn: HashMap<_, usize> = HashMap::new();
    let mut groups = Vec::new();
    let mut order = Vec::new();
    for event in page.paint_order_for_text_runs(&runs) {
        match event {
            PaintEvent::PushOpacity(alpha) => {
                groups.push(alpha);
                order.push(format!("push {alpha}"));
            }
            PaintEvent::PopOpacity => {
                groups.pop().unwrap();
                order.push("pop".into());
            }
            PaintEvent::Box(fragment) => {
                if let Some(name) = id(fragment.node()) {
                    order.push(format!("box {name}"));
                }
            }
            PaintEvent::TextLine(line) => {
                assert!(groups.is_empty(), "paragraph text inside a group");
                for run in runs.iter().filter(|run| run.line == line) {
                    if page.text_run_opacity_group(run).is_none() {
                        *drawn.entry(run_key(run)).or_default() += 1;
                    }
                }
                order.push("text".into());
            }
            PaintEvent::GroupTextLine(line, group) => {
                let name = id(group).unwrap();
                assert_eq!(
                    groups.len(),
                    if name == "inner" { 2 } else { 1 },
                    "{name} text outside its group"
                );
                for run in runs.iter().filter(|run| run.line == line) {
                    if page.text_run_opacity_group(run) == Some(group) {
                        *drawn.entry(run_key(run)).or_default() += 1;
                    }
                }
                order.push(format!("text {name}"));
            }
            PaintEvent::Text(_) => panic!("fragment text in supplied line stream"),
            _ => {}
        }
    }
    assert!(groups.is_empty());
    assert!(drawn.values().all(|&count| count == 1), "{drawn:?}");
    assert_eq!(drawn.len(), runs.len());
    // One entry per text stream, however many lines it has.
    order.dedup_by(|a, b| a == b && a.starts_with("text"));
    assert_eq!(
        order,
        [
            "box p",
            "text",
            "push 0.5",
            "box outer",
            "text outer",
            "box atomic",
            "push 0.25",
            "box inner",
            "text inner",
            "pop",
            "pop",
        ],
    );

    // Without supplied runs, the text fragments go into their groups too.
    let mut depth = 0;
    let mut texts = Vec::new();
    for event in page.paint_order() {
        match event {
            PaintEvent::PushOpacity(_) => depth += 1,
            PaintEvent::PopOpacity => depth -= 1,
            PaintEvent::Text(fragment) => {
                texts.push((dom.parent(fragment.node()).and_then(id), depth));
            }
            _ => {}
        }
    }
    assert_eq!(
        texts,
        [
            (Some("p"), 0),
            (Some("p"), 0),
            (Some("outer"), 1),
            (Some("outer"), 1),
            (Some("inner"), 2),
        ],
    );
}

#[test]
fn display_contents_elements_form_no_opacity_group() {
    let result = lay_out_with_font(
        "<div style='display:contents;opacity:.5'><p>Block</p></div>\
         <p>A<span style='display:contents;opacity:.5'>B</span>C</p>",
        "p {margin:0}",
    );
    let page = result.page(0).unwrap();
    let runs = page.text_runs();
    assert!(!runs.is_empty());
    assert!(
        runs.iter()
            .all(|run| page.text_run_opacity_group(run).is_none())
    );
    for events in [page.paint_order(), page.paint_order_for_text_runs(&runs)] {
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PaintEvent::PushOpacity(_))),
            "{events:?}"
        );
    }
}
