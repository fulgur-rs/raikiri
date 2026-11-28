//! Balanced columns whose content runs past the end of a page. Each page the
//! columns reach fills them to the end of the page, or of the container's
//! content box when it has a definite height, and only the last page balances
//! what is left (CSS Multi-column 1, 8.1). A minimum height still holds the
//! container open below the columns.
//!
//! Each row is a document and where every box with an id, or line of text,
//! lands on each page, as Chromium prints it on pages 180px wide and 160px
//! tall. The containers have two columns 40px wide with a gap of 20px, so the
//! second column starts at x=60.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page of `body`: where the pieces of every box with an id and every
/// line of text start, as `id=x,y` and `text@x,y`, column by column from the
/// top.
fn pages(body: &str) -> Vec<String> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}\
         body{{margin:0;font:20px/20px Ahem}}\
         .mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1}}</style>{body}"
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
    (0..result.page_count())
        .map(|index| {
            let page = result.page(index).unwrap();
            let dom = page.dom();
            let mut placed: Vec<(i32, i32, String)> = page
                .fragments()
                .filter_map(|fragment| {
                    let id = dom.attr(fragment.node(), "id")?;
                    let rect = fragment.rect();
                    let (x, y) = (rect.x.round() as i32, rect.y.round() as i32);
                    Some((x, y, format!("{id}={x},{y}")))
                })
                .collect();
            // Ahem's baseline sits 16px below the top of a 20px line.
            placed.extend(page.text_runs().iter().map(|run| {
                let (x, y) = (
                    run.origin.0.round() as i32,
                    (run.origin.1 - 16.0).round() as i32,
                );
                (x, y, format!("{}@{x},{y}", run.text))
            }));
            placed.sort();
            placed
                .into_iter()
                .map(|(.., text)| text)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// `count` boxes `height` tall with ids from `a`, each with `style` too.
fn boxes(count: u8, height: u32, style: &str) -> String {
    (0..count)
        .map(|index| {
            let id = char::from(b'a' + index);
            format!("<div id={id} style='height:{height}px;{style}'></div>")
        })
        .collect()
}

/// Lines of text `A`, `B`, and so on, one letter each.
fn lines(count: u8) -> String {
    (0..count)
        .map(|index| char::from(b'A' + index).to_string())
        .collect::<Vec<_>>()
        .join("<br>")
}

const AFTER: &str = "<div id=z style='height:10px'></div>";

#[test]
fn auto_height_columns_fill_the_rest_of_the_page_and_balance_on_the_next() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,120 b=0,150 b=60,120 c=60,140",
            "c=0,0 d=0,10 z=0,20 d=60,0",
        ]
    );
}

#[test]
fn a_minimum_height_holds_the_container_open_below_the_columns() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc style='min-height:100px'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,120 b=0,150 b=60,120 c=60,140",
            "c=0,0 d=0,10 z=0,60 d=60,0",
        ]
    );
}

#[test]
fn columns_that_fill_in_turn_fill_the_page_despite_a_minimum_height() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='min-height:100px;column-fill:auto'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        ["a=0,100 b=0,130 c=60,100 d=60,130", "z=0,40"]
    );
}

#[test]
fn only_the_last_page_balances_many_boxes() {
    let body = format!(
        "<div style='height:100px'></div><div class=mc>{}</div>{AFTER}",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,100 b=0,130 c=60,100 d=60,130",
            "e=0,0 f=0,30 g=0,60 h=0,90 i=0,120 z=0,150 \
             j=60,0 k=60,30 l=60,60 m=60,90 n=60,120",
        ]
    );
}

#[test]
fn boxes_in_a_wrapper_fill_the_columns_of_every_page_from_the_first() {
    // The boxes a wrapper holds stand in the piece of the wrapper in their
    // own column, which on the next page starts the first column again.
    let body = format!(
        "<div style='height:100px'></div><div class=mc><div>{}</div></div>{AFTER}",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,100 b=0,130 c=60,100 d=60,130",
            "e=0,0 f=0,30 g=0,60 h=0,90 i=0,120 z=0,150 \
             j=60,0 k=60,30 l=60,60 m=60,90 n=60,120",
        ]
    );
    // In a wrapper inside another.
    let body = format!(
        "<div style='height:100px'></div><div class=mc><div><div>{}</div></div></div>{AFTER}",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,100 b=0,130 c=60,100 d=60,130",
            "e=0,0 f=0,30 g=0,60 h=0,90 i=0,120 z=0,150 \
             j=60,0 k=60,30 l=60,60 m=60,90 n=60,120",
        ]
    );
    // Where `justify-items` puts them in the wrapper.
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc><div style='justify-items:end'>{}</div></div>",
        boxes(14, 30, "width:20px")
    );
    assert_eq!(
        pages(&body),
        [
            "a=20,100 b=20,130 c=80,100 d=80,130",
            "e=20,0 f=20,30 g=20,60 h=20,90 i=20,120 \
             j=80,0 k=80,30 l=80,60 m=80,90 n=80,120",
        ]
    );
    // A wrapper below another box, whose pieces start lower in the column.
    let body = format!(
        "<div style='height:40px'></div>\
         <div class=mc><div id=p style='height:60px'></div><div>{}</div></div>",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "p=0,40 a=0,100 b=0,130 c=60,40 d=60,70 e=60,100 f=60,130",
            "g=0,0 h=0,30 i=0,60 j=0,90 k=60,0 l=60,30 m=60,60 n=60,90",
        ]
    );
    // Inside the wrapper's padding.
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc><div style='padding-left:10px'>{}</div></div>{AFTER}",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=10,100 b=10,130 c=70,100 d=70,130",
            "z=0,150 e=10,0 f=10,30 g=10,60 h=10,90 i=10,120 \
             j=70,0 k=70,30 l=70,60 m=70,90 n=70,120",
        ]
    );
}

#[test]
fn margins_are_truncated_at_the_breaks_of_the_page_filled_columns() {
    let body = format!(
        "<div style='height:100px'></div><div class=mc>{}</div>{AFTER}",
        boxes(5, 30, "margin:10px 0")
    );
    assert_eq!(
        pages(&body),
        ["a=0,110 b=0,150 b=60,100 c=60,130", "d=0,0 z=0,40 e=60,0"]
    );
}

#[test]
fn the_edges_of_the_container_stay_around_the_page_filled_columns() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='padding:10px 0;border:5px solid'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        ["a=5,115 b=5,145 b=65,115 c=65,130", "z=0,30 d=5,0 d=65,0"]
    );
}

#[test]
fn definite_height_columns_fill_each_page_up_to_the_content_box() {
    let body = format!(
        "<div class=mc style='height:300px'>{}</div>",
        boxes(14, 40, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,0 b=0,40 c=0,80 d=0,120 e=60,0 f=60,40 g=60,80 h=60,120",
            "i=0,0 j=0,40 k=0,80 l=60,0 m=60,40 n=60,80",
        ]
    );
}

#[test]
fn definite_height_columns_of_a_paragraph_fill_each_page() {
    let body = format!(
        "<div class=mc style='height:300px'><p style='margin:0'>{}</p></div>",
        lines(20)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,0 B@0,20 C@0,40 D@0,60 E@0,80 F@0,100 G@0,120 H@0,140 \
             I@60,0 J@60,20 K@60,40 L@60,60 M@60,80 N@60,100 O@60,120 P@60,140",
            "Q@0,0 R@0,20 S@60,0 T@60,20",
        ]
    );
}

#[test]
fn content_past_a_definite_height_keeps_the_columns_the_container_laid_out() {
    // The columns cannot hold the boxes up to the content box, so the
    // pages cut the columns the container laid out: each box still shows
    // once, and no two pieces of a page start at the same place.
    let body = format!(
        "<div class=mc style='height:300px'>{}</div>",
        boxes(20, 40, "")
    );
    let pages = pages(&body);
    for id in 'a'..='t' {
        let piece = format!("{id}=");
        assert!(
            pages
                .iter()
                .any(|page| page.split(' ').any(|p| p.starts_with(&piece)))
        );
    }
    for page in &pages {
        let mut places: Vec<&str> = page
            .split(' ')
            .map(|piece| piece.split_once('=').unwrap().1)
            .collect();
        let count = places.len();
        places.sort_unstable();
        places.dedup();
        assert_eq!(places.len(), count, "{page}");
    }
}

#[test]
fn definite_height_columns_of_the_container_s_own_lines_fill_each_page() {
    let body = format!("<div class=mc style='height:300px'>{}</div>", lines(20));
    assert_eq!(
        pages(&body),
        [
            "A@0,0 B@0,20 C@0,40 D@0,60 E@0,80 F@0,100 G@0,120 H@0,140 \
             I@60,0 J@60,20 K@60,40 L@60,60 M@60,80 N@60,100 O@60,120 P@60,140",
            "Q@0,0 R@0,20 S@60,0 T@60,20",
        ]
    );
}

#[test]
fn the_container_s_own_lines_fill_the_rest_of_the_page_and_balance_on_the_next() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc style='orphans:2;widows:2'>{}</div>{AFTER}",
        lines(10)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,120 B@0,140 C@60,120 D@60,140",
            "E@0,0 F@0,20 G@0,40 z=0,60 H@60,0 I@60,20 J@60,40",
        ]
    );
}

#[test]
fn the_edges_of_the_container_stay_around_its_own_page_filled_lines() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='padding:10px 0;border:5px solid'>{}</div>{AFTER}",
        lines(10)
    );
    assert_eq!(
        pages(&body),
        [
            "A@5,115 B@5,135 C@65,115 D@65,135",
            "z=0,75 E@5,0 F@5,20 G@5,40 H@65,0 I@65,20 J@65,40",
        ]
    );
}

#[test]
fn the_container_s_own_lines_fill_every_page_they_run_across() {
    let body = format!(
        "<div style='height:100px'></div><div class=mc>{}</div>{AFTER}",
        lines(26)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,100 B@0,120 C@0,140 D@60,100 E@60,120 F@60,140",
            "G@0,0 H@0,20 I@0,40 J@0,60 K@0,80 L@0,100 M@0,120 N@0,140 \
             O@60,0 P@60,20 Q@60,40 R@60,60 S@60,80 T@60,100 U@60,120 V@60,140",
            "W@0,0 X@0,20 z=0,40 Y@60,0 Z@60,20",
        ]
    );
}

#[test]
fn own_lines_past_a_definite_height_keep_the_columns_the_container_laid_out() {
    // The lines run past the content box, so the pages cut the columns the
    // container laid out: each line still shows once.
    let body = format!(
        "<div style='height:60px'></div><div class=mc style='height:150px'>{}</div>",
        lines(20)
    );
    let pages = pages(&body);
    for line in 'A'..='T' {
        let piece = format!("{line}@");
        let count: usize = pages
            .iter()
            .map(|page| page.split(' ').filter(|p| p.starts_with(&piece)).count())
            .sum();
        assert_eq!(count, 1, "{line}");
    }
}

#[test]
fn own_lines_that_leave_too_few_orphans_on_the_page_start_on_the_next() {
    let body = format!(
        "<div style='height:140px'></div><div class=mc style='orphans:2;widows:2'>{}</div>{AFTER}",
        lines(7)
    );
    assert_eq!(
        pages(&body),
        [
            "",
            "A@0,0 B@0,20 C@0,40 D@0,60 z=0,80 E@60,0 F@60,20 G@60,40",
        ]
    );
}

#[test]
fn an_own_line_taller_than_the_rest_of_the_page_starts_on_the_next() {
    let body = format!(
        "<div style='height:150px'></div><div class=mc>{}</div>{AFTER}",
        lines(6)
    );
    assert_eq!(
        pages(&body),
        ["", "A@0,0 B@0,20 C@0,40 z=0,60 D@60,0 E@60,20 F@60,40"]
    );
}

#[test]
fn the_last_column_of_a_page_gives_its_own_lines_to_the_widows_on_the_next() {
    let body = format!(
        "<div style='height:80px'></div><div class=mc style='orphans:2;widows:2'>{}</div>{AFTER}",
        lines(9)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,80 B@0,100 C@0,120 D@0,140 E@60,80 F@60,100 G@60,120",
            "H@0,0 I@0,20 z=0,40",
        ]
    );
}

#[test]
fn a_minimum_height_holds_open_the_container_of_page_filled_own_lines() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc style='min-height:200px'>{}</div>{AFTER}",
        lines(10)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,120 B@0,140 C@60,120 D@60,140",
            "E@0,0 F@0,20 G@0,40 H@60,0 I@60,20 J@60,40",
            "z=0,0",
        ]
    );
}

#[test]
fn own_lines_in_the_columns_of_another_container_show_once() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='width:180px;column-count:1'><div class=mc>{}</div></div>",
        lines(10)
    );
    let pages = pages(&body);
    for line in 'A'..='J' {
        let piece = format!("{line}@");
        let count: usize = pages
            .iter()
            .map(|page| page.split(' ').filter(|p| p.starts_with(&piece)).count())
            .sum();
        assert_eq!(count, 1, "{line}");
    }
}

#[test]
fn own_lines_whose_columns_leave_too_few_widows_on_the_page_start_on_the_next() {
    let body = format!(
        "<div style='height:120px'></div>\
         <div class=mc style='width:160px;column-count:3;widows:3'>{}</div>{AFTER}",
        lines(16)
    );
    assert_eq!(
        pages(&body),
        [
            "",
            "A@0,0 B@0,20 C@0,40 D@0,60 E@0,80 F@0,100 z=0,120 \
             G@60,0 H@60,20 I@60,40 J@60,60 K@60,80 L@60,100 \
             M@120,0 N@120,20 O@120,40 P@120,60",
        ]
    );
}

#[test]
fn the_last_page_balances_own_lines_with_widows_in_every_column() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='width:160px;column-count:3;widows:3'>{}</div>{AFTER}",
        lines(19)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,100 B@0,120 C@0,140 D@60,100 E@60,120 F@60,140 G@120,100 H@120,120 I@120,140",
            "J@0,0 K@0,20 L@0,40 M@0,60 z=0,80 N@60,0 O@60,20 P@60,40 Q@120,0 R@120,20 S@120,40",
        ]
    );
}

#[test]
fn boxes_with_inline_margins_fill_each_page_before_the_next_one() {
    // Margins on the left or right only move a box along its column, so the
    // columns fill page by page as they do for boxes with none (Chromium).
    let pages_of = |style: &str, left: i32| {
        let at = |id: u8, x: i32, y: i32| format!("{}={},{y}", char::from(b'a' + id), x + left);
        let page = |first: u8, rows: u8| {
            (0..2)
                .flat_map(|column| {
                    (0..rows).map(move |row| (first + column * rows + row, column, row))
                })
                .map(|(id, column, row)| at(id, i32::from(column) * 60, i32::from(row) * 40))
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            pages(&format!("<div class=mc>{}</div>", boxes(20, 40, style))),
            [page(0, 4), page(8, 4), page(16, 2)],
            "{style}"
        );
    };
    pages_of("margin-left:2px", 2);
    pages_of("margin-left:4px;margin-right:6px", 4);
    pages_of("width:20px;margin:0 auto", 10);
    // `justify-self` moves a box along the room its margins leave.
    assert_eq!(
        pages(&format!(
            "<div class=mc>{}</div>",
            boxes(4, 40, "width:20px;justify-self:end;margin-right:3px")
        )),
        ["a=17,0 b=17,40 c=77,0 d=77,40"]
    );
    // Right to left, a narrower box keeps to the right of its column, past
    // its right margin.
    let rtl: String = (0..4)
        .map(|index| {
            let margin = if index % 2 == 1 {
                "margin-right:3px"
            } else {
                ""
            };
            let id = char::from(b'a' + index);
            format!("<div id={id} style='height:40px;width:20px;{margin}'></div>")
        })
        .collect();
    assert_eq!(
        pages(&format!("<div class=mc style=direction:rtl>{rtl}</div>")),
        ["d=17,40 c=20,0 b=77,40 a=80,0"]
    );
}

#[test]
fn right_to_left_columns_fill_each_page_with_boxes_narrower_than_them() {
    // A narrower box keeps to the right of a right-to-left column, or of the
    // content box of a wrapper, and the columns fill page by page, from the
    // right, as Chromium prints them.
    let pages_of = |body: &str, left: i32| {
        let at = |id: u8, x: i32, y: i32| format!("{}={},{y}", char::from(b'a' + id), x + left);
        let page = |first: u8, rows: u8| {
            (0..2u8)
                .flat_map(|column| (0..rows).map(move |row| (column, row)))
                .map(|(column, row)| {
                    // The first boxes of a page fill its right column.
                    let id = first + (1 - column) * rows + row;
                    at(id, i32::from(column) * 60, i32::from(row) * 40)
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            pages(&format!("<div class=mc style=direction:rtl>{body}</div>")),
            [page(0, 4), page(8, 4), page(16, 2)],
            "{body}"
        );
    };
    pages_of(&boxes(20, 40, "width:20px"), 20);
    pages_of(&boxes(20, 40, "width:20px;margin-right:3px"), 17);
    pages_of(&boxes(20, 40, "width:20px;margin:0 auto"), 10);
    pages_of(
        &format!(
            "<div style=width:30px>{}</div>",
            boxes(20, 40, "width:20px")
        ),
        20,
    );
    pages_of(
        &format!(
            "<div style='width:30px;padding:0 2px'>{}</div>",
            boxes(20, 40, "")
        ),
        8,
    );
    pages_of(
        &format!(
            "<div style='width:30px;border-right:4px solid'>{}</div>",
            boxes(20, 40, "width:20px;margin-left:1px")
        ),
        16,
    );
}

#[test]
fn boxes_with_inline_margins_in_a_wrapper_fill_each_page_in_column_order() {
    // Inside a wrapper, margins on the left or right move a box along the
    // wrapper's content box, and the columns still fill page by page
    // (Chromium).
    assert_eq!(
        pages(&format!(
            "<div class=mc><div style='width:30px;border-left:4px solid'>{}</div></div>",
            boxes(20, 40, "width:20px;margin-right:1px")
        )),
        [
            "a=4,0 b=4,40 c=4,80 d=4,120 e=64,0 f=64,40 g=64,80 h=64,120",
            "i=4,0 j=4,40 k=4,80 l=4,120 m=64,0 n=64,40 o=64,80 p=64,120",
            "q=4,0 r=4,40 s=64,0 t=64,40",
        ]
    );
    assert_eq!(
        pages(&format!(
            "<div class=mc><div>{}</div></div>",
            boxes(20, 40, "width:20px;margin:0 auto")
        )),
        [
            "a=10,0 b=10,40 c=10,80 d=10,120 e=70,0 f=70,40 g=70,80 h=70,120",
            "i=10,0 j=10,40 k=10,80 l=10,120 m=70,0 n=70,40 o=70,80 p=70,120",
            "q=10,0 r=10,40 s=70,0 t=70,40",
        ]
    );
    // On one page, past the wrapper's padding.
    assert_eq!(
        pages(&format!(
            "<div class=mc><div style='padding-left:2px'>{}</div></div>",
            boxes(4, 40, "margin-left:3px")
        )),
        ["a=5,0 b=5,40 c=65,0 d=65,40"]
    );
    assert_eq!(
        pages(&format!(
            "<div class=mc style=direction:rtl><div style='padding-right:2px'>{}</div></div>",
            boxes(4, 40, "width:20px;margin-right:3px")
        )),
        ["c=15,0 d=15,40 a=75,0 b=75,40"]
    );
}
