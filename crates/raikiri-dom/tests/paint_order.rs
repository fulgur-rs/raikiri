//! A page's body content listed in paint order, after a full layout.

use raikiri_dom::PaintEvent;
use raikiri_dom::paint_rules::{PositionedFrame, PositionedQueue};
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeId, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// One label per event of page `page`: `Box(id)`, `Text(parent-id)`,
/// `Replaced(id)`, `PushClip`, `PopClip`, `PushOpacity` or `PopOpacity`.
/// Elements without an id are labelled with their tag name.
fn order(html: &str, page: u32) -> Vec<String> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!("<!doctype html><style>body{{font:10px/10px Ahem}}</style>{html}");
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
    let page = result
        .pages()
        .find(|p| p.index() == page)
        .expect("page in range");
    let dom = page.dom();
    let label = |node: NodeId| -> String {
        dom.attr(node, "id")
            .or_else(|| dom.local_name(node))
            .unwrap_or("?")
            .to_owned()
    };
    let (document, cascade, _, _) = page.paint_inputs();
    document
        .page_paint_order(cascade, page.index(), page.name())
        .into_iter()
        .map(|event| match event {
            PaintEvent::PushClip(..) => "PushClip".to_owned(),
            PaintEvent::PopClip => "PopClip".to_owned(),
            PaintEvent::PushOpacity(_) => "PushOpacity".to_owned(),
            PaintEvent::PopOpacity => "PopOpacity".to_owned(),
            PaintEvent::Box(f) => format!("Box({})", label(f.node())),
            PaintEvent::Text(f) => {
                let parent = dom.parent(f.node()).expect("text has a parent");
                format!("Text({})", label(parent))
            }
            PaintEvent::Replaced(f) => format!("Replaced({})", label(f.node())),
            _ => "Other".to_owned(),
        })
        .collect()
}

fn position(events: &[String], label: &str) -> usize {
    events
        .iter()
        .position(|e| e == label)
        .unwrap_or_else(|| panic!("{label} missing from {events:?}"))
}

#[test]
fn absolute_box_comes_after_preceding_paragraph_text() {
    let events = order(
        "<body><p id=p>text</p><div id=a style='position:absolute;top:0;left:0;\
         width:50px;height:50px;background:red'></div></body>",
        0,
    );
    let text = position(&events, "Text(p)");
    let abs = position(&events, "Box(a)");
    assert!(text < abs, "{events:?}");
}

#[test]
fn negative_z_index_paints_before_in_flow_siblings() {
    let events = order(
        "<body><p id=p>text</p><div id=n style='position:relative;z-index:-1;height:10px'></div></body>",
        0,
    );
    let neg = position(&events, "Box(n)");
    let para = position(&events, "Box(p)");
    assert!(neg < para, "{events:?}");
}

#[test]
fn negative_inline_content_paints_below_paragraph_blocks_and_lines_above() {
    // The negative span paints first, then the block inside the paragraph,
    // then the paragraph's other text, whether the paragraph defers its
    // content or forms the stacking context itself.
    for style in ["", "position:relative;z-index:0"] {
        let events = order(
            &format!(
                "<div id=m style='{style}'>over<span id=n style='position:relative;z-index:-1'>x</span>\
                 <div id=k style='background:gray;height:10px'></div>tail</div>"
            ),
            0,
        );
        let (n, k, m) = (
            position(&events, "Text(n)"),
            position(&events, "Box(k)"),
            position(&events, "Text(m)"),
        );
        assert!(n < k && k < m, "{style}: {events:?}");
    }
}

#[test]
fn negative_inline_content_inside_a_display_contents_element_paints_below_paragraph_blocks() {
    // A `display: contents` element has no box, so its position and
    // `z-index` form no stacking context and the negative span inside it
    // still paints below the block.
    let events = order(
        "<div id=m>over<span style='display:contents;position:relative;z-index:0'>\
         <span id=n style='position:relative;z-index:-1'>x</span></span>\
         <div id=k style='background:gray;height:10px'></div>tail</div>",
        0,
    );
    let (n, k, m) = (
        position(&events, "Text(n)"),
        position(&events, "Box(k)"),
        position(&events, "Text(m)"),
    );
    assert!(n < k && k < m, "{events:?}");
}

#[test]
fn negative_inline_content_inside_an_inline_stacking_context_stays_in_it() {
    // The negative span is inside a span that forms a stacking context, so
    // it stays in the lines with that span, after the block.
    let events = order(
        "<div id=m>over<span id=s style='position:relative;z-index:0'>\
         <span id=n style='position:relative;z-index:-1'>x</span></span>\
         <div id=k style='background:gray;height:10px'></div></div>",
        0,
    );
    let (k, m, n) = (
        position(&events, "Box(k)"),
        position(&events, "Text(m)"),
        position(&events, "Text(n)"),
    );
    assert!(k < m && m < n, "{events:?}");
}

#[test]
fn opacity_and_overflow_nest_around_children() {
    let events = order(
        "<body><div id=o style='opacity:.5;overflow:hidden;height:40px'><p id=c>x</p></div></body>",
        0,
    );
    let start = position(&events, "PushOpacity");
    assert_eq!(
        &events[start..start + 7],
        [
            "PushOpacity",
            "Box(o)",
            "PushClip",
            "Box(c)",
            "Text(c)",
            "PopClip",
            "PopOpacity"
        ],
        "{events:?}"
    );
}

#[test]
fn events_are_balanced_on_every_page() {
    // Three pages; the opacity element spans the first two.
    let html = "<style>@page{size:300px 100px;margin:0}</style><body>\
        <div style='opacity:.5;overflow:hidden'><p style='height:150px'>a</p></div>\
        <p style='height:80px'>b</p></body>";
    for page in 0..3 {
        let mut depth = 0i32;
        for e in order(html, page) {
            match e.as_str() {
                "PushClip" | "PushOpacity" => depth += 1,
                "PopClip" | "PopOpacity" => {
                    depth -= 1;
                    assert!(depth >= 0, "page {page}");
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "page {page}");
    }
}

#[test]
fn replaced_follows_its_box() {
    let events = order("<body><img id=i style='width:10px;height:10px'></body>", 0);
    let b = position(&events, "Box(i)");
    assert_eq!(events[b + 1], "Replaced(i)", "{events:?}");
}

#[test]
fn inline_element_boxes_come_before_paragraph_text() {
    let events = order(
        "<body><p id=p>a <span id=s style='background:red'>b</span> c</p></body>",
        0,
    );
    let span = position(&events, "Box(s)");
    let text = position(&events, "Text(p)");
    assert!(span < text, "{events:?}");
    assert!(events.contains(&"Text(s)".to_owned()), "{events:?}");
}

#[test]
fn every_event_fragment_is_a_page_fragment() {
    let html = "<body><p id=p>a <span>b</span></p><img style='width:4px;height:4px'>\
        <div style='opacity:.5;overflow:hidden'><p>x</p></div></body>";
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
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
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let fragments: Vec<_> = document
        .page_fragments(page.index())
        .map(|f| (f.node(), f.kind(), f.fragment_index()))
        .collect();
    for event in document.page_paint_order(cascade, page.index(), page.name()) {
        let f = match event {
            PaintEvent::Box(f) | PaintEvent::Text(f) | PaintEvent::Replaced(f) => f,
            _ => continue,
        };
        assert!(
            fragments.contains(&(f.node(), f.kind(), f.fragment_index())),
            "{:?} is not a page fragment",
            f.node()
        );
    }
}

#[test]
fn multicol_containers_are_reported_as_approximations() {
    let html = "<!doctype html><body><div id=m style='column-count:2'><p>a</p><p>b</p></div>\
        <div style='column-count:1'><p>c</p></div></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let found: Vec<_> = document
        .paint_order_approximations(cascade)
        .into_iter()
        .map(|(node, reason)| (page.dom().attr(node, "id"), reason))
        .collect();
    assert_eq!(
        found,
        [(Some("m"), "legacy column clipping may be incomplete")]
    );
}

#[test]
fn nested_multicol_reports_only_the_outermost() {
    let html = "<!doctype html><body><div id=outer style='columns:2'>\
        <div id=inner style='columns:2'><p>a</p><p>b</p></div><p>c</p></div></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let found: Vec<_> = document
        .paint_order_approximations(cascade)
        .into_iter()
        .map(|(node, _)| page.dom().attr(node, "id"))
        .collect();
    assert_eq!(found, [Some("outer")]);
}

#[test]
fn multicol_inside_display_none_is_not_reported() {
    let html = "<!doctype html><body><div style='display:none'>\
        <div style='columns:2'><p>a</p><p>b</p></div></div><p>c</p></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    assert!(document.paint_order_approximations(cascade).is_empty());
}

#[test]
fn hidden_and_non_rendered_elements_are_not_listed() {
    let events = order(
        "<body><script id=s>x</script><div id=h style='display:none'>x</div>\
         <p id=p>a<span id=hs style='display:none'>b</span><script id=ps>c</script></p></body>",
        0,
    );
    for hidden in ["Box(s)", "Box(h)", "Box(hs)", "Box(ps)"] {
        assert!(
            !events.iter().any(|e| e == hidden),
            "{hidden} in {events:?}"
        );
    }
    position(&events, "Text(p)");
}

/// A text node laid out as an anonymous flex item is a paragraph of its own.
#[test]
fn flex_item_text_is_its_own_paragraph() {
    let events = order("<body><div id=f style='display:flex'>text</div></body>", 0);
    let flex = position(&events, "Box(f)");
    let text = position(&events, "Text(f)");
    assert!(flex < text, "{events:?}");
}

#[test]
fn a_page_outside_the_layout_lists_nothing() {
    let doc =
        parse_html_with_resources(b"<p>a</p>".as_slice(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    assert!(document.page_paint_order(cascade, 99, None).is_empty());
}

#[test]
fn stacking_contexts_in_different_items_paint_in_stack_level_order() {
    // CSS 2.1 Appendix E step 9: positioned descendants paint in z-index
    // order within the nearest stacking context, whatever box holds them.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div style='display:flex'><div><div id=a style='{item};z-index:10'></div>\
             <div id=b style='{item};z-index:20'></div></div>\
             <div><div id=c style='{item};z-index:15'></div>\
             <div style='position:relative'><p>x</p></div>\
             <div style='position:relative;opacity:0.5'></div></div></div>"
        ),
        0,
    );
    let (a, b, c) = (
        position(&events, "Box(a)"),
        position(&events, "Box(b)"),
        position(&events, "Box(c)"),
    );
    assert!(a < c && c < b, "{events:?}");
}

#[test]
fn a_box_moved_out_of_a_clip_keeps_its_clip() {
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div style='overflow:hidden'>\
             <div id=a style='{item};z-index:10'></div>\
             <div id=b style='{item};z-index:20'></div>\
             <div id=d style='{item};z-index:20'></div></div>\
             <div><div id=c style='{item};z-index:15'></div></div></div>"
        ),
        0,
    );
    let start = position(&events, "Box(a)") - 1;
    assert_eq!(
        events[start..],
        [
            "PushClip", "Box(a)", "PopClip", "Box(c)", "PushClip", "Box(b)", "Box(d)", "PopClip"
        ],
    );
}

#[test]
fn a_z_indexed_flex_item_paints_among_positioned_descendants_by_level() {
    // CSS Flexbox 1 §4.3: a flex item with an integer z-index is a stacking
    // context at that level even when it is not positioned.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div style='display:flex'><div><div id=a style='{item};z-index:10'></div>\
             <div id=b style='{item};z-index:20'></div></div>\
             <div id=c style='opacity:0.5;z-index:15;width:10px;height:10px;background:red'>\
             </div></div>"
        ),
        0,
    );
    let (a, b, c) = (
        position(&events, "Box(a)"),
        position(&events, "Box(b)"),
        position(&events, "Box(c)"),
    );
    assert!(a < c && c < b, "{events:?}");
}

#[test]
fn a_z_auto_positioned_box_does_not_carry_its_descendants_below_others() {
    // A positioned box with `z-index: auto` forms no stacking context, so
    // the box with `z-index: 20` inside it still paints above `z-index: 10`.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=a style='{item};z-index:10'></div></div>\
             <div><div id=p style='position:relative'>\
             <div id=b style='{item};z-index:20'></div></div></div>"
        ),
        0,
    );
    assert!(
        position(&events, "Box(a)") < position(&events, "Box(b)"),
        "{events:?}"
    );
}

#[test]
fn a_z_auto_positioned_box_keeps_descendants_behind_display_contents() {
    // A `display: contents` wrapper generates no box, but its children still
    // generate boxes in the enclosing stacking context.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=a style='{item};z-index:10'></div></div>\
             <div><div id=p style='position:relative'><div style='display:contents'>\
             <div id=b style='{item};z-index:20'></div></div></div></div>"
        ),
        0,
    );
    assert!(
        position(&events, "Box(a)") < position(&events, "Box(b)"),
        "{events:?}"
    );
}

#[test]
fn boxes_beside_a_z_auto_positioned_box_still_sort() {
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=x style='{item};z-index:20'></div></div>\
             <div><div id=y style='{item};z-index:10'></div></div>\
             <div><div id=p style='position:relative'>\
             <div id=z style='{item};z-index:30'></div></div></div>"
        ),
        0,
    );
    let (x, y, z) = (
        position(&events, "Box(x)"),
        position(&events, "Box(y)"),
        position(&events, "Box(z)"),
    );
    assert!(y < x && x < z, "{events:?}");
}

#[test]
fn boxes_inside_a_z_auto_positioned_box_sort_with_the_boxes_around_it() {
    // CSS 2.1 Appendix E step 9: the boxes inside a positioned box with
    // `z-index: auto` belong to the enclosing stacking context, so they
    // interleave by level with the boxes before and after it.
    let item = "position:relative;width:10px;height:10px;background:red";
    for (inner, expected) in [
        ("30", ["Box(p)", "Box(y)", "Box(x)", "Box(z)"]),
        ("15", ["Box(p)", "Box(y)", "Box(z)", "Box(x)"]),
    ] {
        let events = order(
            &format!(
                "<div><div id=x style='{item};z-index:20'></div></div>\
                 <div><div id=p style='position:relative;height:10px;background:red'>\
                 <div id=z style='{item};z-index:{inner}'></div></div></div>\
                 <div><div id=y style='{item};z-index:10'></div></div>"
            ),
            0,
        );
        let mut at: Vec<_> = expected.iter().map(|id| position(&events, id)).collect();
        let listed = at.clone();
        at.sort_unstable();
        assert_eq!(at, listed, "z-index {inner}: {events:?}");
    }
}

#[test]
fn a_queue_ignores_boxes_handed_on_before_it_hands_out_a_box() {
    // Only a box the queue handed out can hand boxes on; boxes offered
    // before that have no place to sort at and are dropped.
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources("<!doctype html><div></div>".as_bytes(), &resources)
        .expect("parse");
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
    let page = result.pages().next().expect("a page");
    let (document, cascade, _, _) = page.paint_inputs();
    let mut queue = PositionedQueue::new(document, cascade, Vec::new(), |f: &PositionedFrame| *f);
    queue.hand_on(document, cascade, vec![PositionedFrame::Box(0)], |f| *f);
    assert!(queue.next(PositionedFrame::Close).is_empty());
}

#[test]
fn boxes_handed_on_twice_keep_tree_order_within_a_level() {
    // `a` comes before `b` in the tree, and both have level 10, so `a`
    // paints first although it reaches the layer through one more
    // `z-index: auto` box.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=x style='{item};z-index:20'></div></div>\
             <div><div id=p style='position:relative'><div id=q style='position:relative'>\
             <div id=a style='{item};z-index:10'></div></div>\
             <div id=b style='{item};z-index:10'></div></div></div>"
        ),
        0,
    );
    let (a, b, x) = (
        position(&events, "Box(a)"),
        position(&events, "Box(b)"),
        position(&events, "Box(x)"),
    );
    assert!(a < b && b < x, "{events:?}");
}

#[test]
fn boxes_inside_many_z_auto_positioned_boxes_sort_together() {
    // Each `z-index: auto` box hands on its box as it paints; all of them
    // still end up in one order by level, the lowest level first.
    let item = "position:relative;width:1px;height:1px;background:red";
    let count = 200;
    let html: String = (0..count)
        .map(|n| {
            format!(
                "<div style='position:relative'><div id=b{n} style='{item};z-index:{}'></div></div>",
                count - n
            )
        })
        .collect();
    let events = order(&html, 0);
    let at: Vec<_> = (0..count)
        .rev()
        .map(|n| position(&events, &format!("Box(b{n})")))
        .collect();
    assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{events:?}");
}

#[test]
fn boxes_inside_nested_z_auto_positioned_boxes_keep_their_clip_and_sort() {
    // The boxes handed on by a nested `z-index: auto` box join the others
    // inside the clip of the block that holds them.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div style='overflow:hidden'>\
             <div id=p style='position:relative'><div id=q style='position:relative'>\
             <div id=a style='{item};z-index:30'></div></div>\
             <div id=b style='{item};z-index:5'></div></div></div>\
             <div><div id=c style='{item};z-index:20'></div></div></div>"
        ),
        0,
    );
    let start = position(&events, "Box(p)") - 1;
    assert_eq!(
        events[start..],
        [
            "PushClip", "Box(p)", "Box(q)", "Box(b)", "PopClip", "Box(c)", "PushClip", "Box(a)",
            "PopClip"
        ],
    );
}

#[test]
fn a_z_auto_positioned_box_holding_a_z_indexed_flex_item_stays_in_place() {
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=a style='{item};z-index:10'></div></div>\
             <div><div style='position:relative'><div style='display:flex'>\
             <div id=f style='z-index:20;width:10px;height:10px;background:red'></div>\
             </div></div></div>"
        ),
        0,
    );
    assert!(
        position(&events, "Box(a)") < position(&events, "Box(f)"),
        "{events:?}"
    );
}

#[test]
fn a_z_auto_positioned_box_holding_boxes_it_paints_stays_in_place() {
    // A box with a negative level, and a positioned box in a paragraph's
    // lines or behind a clipping flex container, paint with the
    // `z-index: auto` box, which keeps its place among the others.
    let item = "position:relative;width:10px;height:10px;background:red";
    for inner in [
        format!("<div style='{item};z-index:-1'></div><div id=f style='{item};z-index:20'></div>"),
        "<span id=f style='position:relative;z-index:20;background:red'>x</span>".to_string(),
        format!(
            "<div style='display:flex;overflow:hidden'><div>\
             <div id=f style='{item};z-index:20'></div></div></div>"
        ),
    ] {
        let events = order(
            &format!(
                "<div><div id=a style='{item};z-index:10'></div></div>\
                 <div><div style='position:relative'>{inner}</div></div>"
            ),
            0,
        );
        assert!(
            position(&events, "Box(a)") < position(&events, "Box(f)"),
            "{inner}: {events:?}"
        );
    }
}

#[test]
fn a_fixed_box_with_z_auto_sorts_as_a_stacking_context() {
    // A fixed box forms a stacking context even with `z-index: auto`, so it
    // moves as a whole at level zero.
    let item = "position:relative;width:10px;height:10px;background:red";
    let events = order(
        &format!(
            "<div><div id=x style='{item};z-index:20'></div></div>\
             <div><div id=f style='position:fixed'>\
             <div style='{item};z-index:30'></div></div></div>\
             <div><div id=y style='{item};z-index:10'></div></div>"
        ),
        0,
    );
    let (f, x, y) = (
        position(&events, "Box(f)"),
        position(&events, "Box(x)"),
        position(&events, "Box(y)"),
    );
    assert!(f < y && y < x, "{events:?}");
}

#[test]
fn hidden_and_nested_levels_do_not_stop_sorting() {
    // A `display: none` box paints nothing, and a box inside a nested
    // stacking context (including a flex item with `z-index: 0` and a sticky
    // box) is ordered within it, so neither keeps the boxes around the
    // `z-index: auto` box from sorting.
    let item = "position:relative;width:10px;height:10px;background:red";
    for inner in [
        format!("<div style='display:none'><div style='{item};z-index:30'></div></div>"),
        format!("<div style='opacity:0.5'><div style='{item};z-index:30'></div></div>"),
        format!(
            "<div style='display:flex'><div style='z-index:0'>\
             <div style='{item};z-index:30'></div></div></div>"
        ),
        format!("<div style='position:sticky'><div style='{item};z-index:30'></div></div>"),
    ] {
        let events = order(
            &format!(
                "<div><div id=x style='{item};z-index:20'></div></div>\
                 <div><div style='position:relative'>{inner}</div></div>\
                 <div><div id=y style='{item};z-index:10'></div></div>"
            ),
            0,
        );
        assert!(
            position(&events, "Box(y)") < position(&events, "Box(x)"),
            "{inner}: {events:?}"
        );
    }
}
