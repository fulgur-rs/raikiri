//! Where the columns after a spanner land when a forced page break inside
//! the spanner continues it on the next page, in a multicol container whose
//! children are boxes. The columns after a spanner start below it (CSS
//! Multi-column Layout 1, 6.1), so they follow it to the next page, with the
//! backgrounds and the column rules that go with them. The positions are the
//! ones Chromium prints for the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// `body` laid out on 100x100 pages with no margin and 10px Ahem lines 10px
/// apart.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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

/// Every word of `body` as `word@page:x,y`, the left and top of its glyphs on
/// its page, sorted.
fn words(body: &str) -> String {
    let result = lay_out(body);
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("a page");
        for run in page.text_runs() {
            let mut x = run.origin.0;
            let y = (run.origin.1 - ASCENT).round();
            for word in run.text.split(' ') {
                if !word.trim().is_empty() {
                    words.push(format!("{}@{index}:{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.sort();
    words.join(" ")
}

const COLUMNS: &str = "columns:2;column-gap:0;width:100px";

/// A two-column container holding a line, a spanner holding `spanner`, and
/// then `after`, followed by a line.
fn spanned(spanner: &str, after: &str) -> String {
    words(&format!(
        "<div style='{COLUMNS}'><div>A</div><div style='column-span:all'>{spanner}</div>\
         {after}</div><div>X</div>"
    ))
}

/// The words of `expected`, sorted as [`words`] gives them.
fn sorted(expected: &str) -> String {
    let mut words: Vec<_> = expected.split(' ').collect();
    words.sort();
    words.join(" ")
}

/// A spanner's content with a forced page break before its second line.
const BREAK_BEFORE: &str = "<div>S</div><div style='break-before:page'>R</div>";

#[test]
fn the_columns_after_a_spanner_follow_its_forced_break() {
    let cases = [
        (
            BREAK_BEFORE,
            "<div>C</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div style='break-after:page'>S</div><div>R</div>",
            "<div>C</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div><div>S</div><div style='break-before:page'>R</div></div>",
            "<div>C</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div>S</div><div style='break-before:page;margin-top:10px'>R</div>",
            "<div>C</div>",
            "A@0:0,0 S@0:0,10 R@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div>S</div><div style='break-before:page'>R</div>\
             <div style='break-before:page'>Q</div>",
            "<div>C</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 Q@2:0,0 C@2:0,10 X@2:0,20",
        ),
        (
            BREAK_BEFORE,
            "<div>C</div><div>D</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 D@1:50,10 X@1:0,20",
        ),
        (
            BREAK_BEFORE,
            "<div>C</div><div style='column-span:all'>T</div><div>D</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 T@1:0,20 D@1:0,30 X@1:0,40",
        ),
        (
            BREAK_BEFORE,
            "<span><div>C</div></span>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            BREAK_BEFORE,
            "<div style='display:contents'><div>C</div></div>\
             <div style='display:none'><div>H</div></div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 X@1:0,20",
        ),
    ];
    for (spanner, after, expected) in cases {
        assert_eq!(
            spanned(spanner, after),
            sorted(expected),
            "{spanner} {after}"
        );
    }
}

#[test]
fn a_later_spanner_with_its_own_forced_break_moves_again() {
    assert_eq!(
        spanned(
            BREAK_BEFORE,
            "<div><div>C</div></div><div style='column-span:all'>\
             <div>T</div><div style='break-before:page'>U</div></div><div>D</div>",
        ),
        sorted("A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,10 T@1:0,20 U@2:0,0 D@2:0,10 X@2:0,20")
    );
}

#[test]
fn the_backgrounds_of_the_spanner_and_its_container_continue_on_the_next_page() {
    let result = lay_out(&format!(
        "<div style='{COLUMNS};background:silver'><div>A</div>\
         <div style='column-span:all;background:green'>{BREAK_BEFORE}</div>\
         <div>C</div></div><div>X</div>"
    ));
    // The boxes 100px wide on each page: the root, the body, the container
    // and the spanner, then the boxes of its lines.
    let wide = |index: u32| -> Vec<(f32, f32)> {
        result
            .page(index)
            .expect("a page")
            .fragments()
            .filter(|fragment| {
                fragment.kind() == FragmentKind::Box && fragment.rect().width == 100.0
            })
            .map(|fragment| (fragment.rect().y, fragment.rect().height))
            .collect()
    };
    let first = wide(0);
    assert!(first.len() >= 4, "{first:?}");
    assert_eq!(first[2..4], [(0.0, 100.0), (10.0, 90.0)], "{first:?}");
    let second = wide(1);
    assert!(second.len() >= 4, "{second:?}");
    assert_eq!(second[2..4], [(0.0, 20.0), (0.0, 10.0)], "{second:?}");
}

/// The page, top and height of every column rule in `body`.
fn rules(body: &str) -> Vec<(u32, f32, f32)> {
    let result = lay_out(body);
    (0..result.page_count())
        .flat_map(|index| {
            let page = result.page(index).expect("a page");
            page.paint_order()
                .iter()
                .filter_map(|event| match event {
                    PaintEvent::ColumnRule(rule) => Some((index, rule.rect.y, rule.rect.height)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_column_rules_after_the_spanner_move_with_its_columns() {
    // A margin as negative as the spanner is tall, or more, starts the
    // columns after it at its top or above, and Chromium leaves them on the
    // first page. A negative block-start margin pulls the spanner up over the
    // columns before it, which stay.
    for (margins, page, top) in [
        ("margin-bottom:0", 1, 10.0),
        ("margin-bottom:-5px", 1, 5.0),
        ("margin-bottom:-20px", 0, 90.0),
        ("margin-bottom:-30px", 0, 80.0),
        ("margin-top:-5px", 1, 10.0),
    ] {
        assert_eq!(
            rules(&format!(
                "<div style='{COLUMNS};column-rule:2px solid'><div>A</div><div>B</div>\
                 <div style='column-span:all;{margins}'>{BREAK_BEFORE}</div>\
                 <div>C</div><div>D</div></div>"
            )),
            [(0, 0.0, 10.0), (page, top, 10.0)],
            "{margins}"
        );
    }
}

#[test]
fn a_spanner_that_moves_whole_keeps_its_height() {
    let body = format!(
        "<div style='{COLUMNS};background:silver'><div>A</div>\
         <div style='column-span:all;background:green;break-before:page'><div>S</div></div>\
         <div>C</div></div><div>X</div>"
    );
    assert_eq!(words(&body), sorted("A@0:0,0 S@1:0,0 C@1:0,10 X@1:0,20"));
    // The container and then the spanner, on the page the spanner moved to.
    let result = lay_out(&body);
    let second: Vec<_> = result
        .page(1)
        .expect("a page")
        .fragments()
        .filter(|fragment| fragment.kind() == FragmentKind::Box && fragment.rect().width == 100.0)
        .map(|fragment| (fragment.rect().y, fragment.rect().height))
        .collect();
    assert!(second.len() >= 4, "{second:?}");
    assert_eq!(second[2..4], [(0.0, 20.0), (0.0, 10.0)], "{second:?}");
}

#[test]
fn the_rules_of_columns_that_run_onto_the_spanners_page_stay_beside_them() {
    // Two columns of twelve lines fill the first page and two more rows of
    // the second, above the spanner that breaks again.
    let lines: String = (1..=24).map(|line| format!("<div>L{line}</div>")).collect();
    assert_eq!(
        rules(&format!(
            "<div style='{COLUMNS};column-rule:2px solid'>{lines}\
             <div style='column-span:all'>{BREAK_BEFORE}</div>\
             <div>C</div><div>D</div></div>"
        )),
        [(0, 0.0, 100.0), (1, 0.0, 20.0), (2, 10.0, 10.0)]
    );
}
