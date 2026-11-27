//! Where a multicol container places a scroll container whose height a
//! maximum or an aspect ratio sets, one that holds absolutely positioned
//! boxes it contains, and a sticky one, which the columns place like a
//! relatively positioned one. Such a scroll container is monolithic (CSS
//! Fragmentation 3, 4.1), so a column end never cuts it: it moves to the
//! next column when it does not fit below what comes before it. The
//! positions are the ones Chromium gives the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;

/// `body` laid out on an 800x600 page with 10px Ahem lines 10px apart.
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
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
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

/// The boxes with an id in `body`, as `id=x,y,width,height` in fragment
/// order, and where the glyph `X` is painted, as `X@x,y`.
fn boxes(body: &str) -> String {
    let result = lay_out(body);
    let page = result.page(0).expect("one page");
    assert!(result.page(1).is_none(), "the content fits one page");
    let dom = page.dom();
    let mut out: Vec<String> = page
        .fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some(format!(
                "{id}={},{},{},{}",
                rect.x, rect.y, rect.width, rect.height
            ))
        })
        .collect();
    for run in page.text_runs() {
        let origin = run
            .transform
            .map_or(run.origin, |transform| transform.apply(run.origin));
        if run.text.trim() == "X" {
            out.push(format!(
                "X@{},{}",
                origin.0.round(),
                (origin.1 - ASCENT).round()
            ));
        }
    }
    out.join(" ")
}

/// A definite-height container of two columns 100px wide that fill in turn,
/// holding `content`.
fn filled(content: &str) -> String {
    boxes(&format!(
        "<div style='columns:2;width:200px;height:100px;column-gap:0;column-fill:auto'>\
         {content}</div>"
    ))
}

/// The box 400px square that a scroll container scrolls.
const LARGE: &str = "<div style='width:400px;height:400px'></div>";

/// An 80px box that leaves too little of the first column for what follows.
const TALL: &str = "<div style='height:80px'></div>";

#[test]
fn a_scroll_container_with_a_maximum_height_stays_in_one_column() {
    assert_eq!(
        filled(&format!(
            "<div id=s style='overflow:auto;max-height:60px'>X{LARGE}</div>"
        )),
        "s=0,0,100,60 X@0,0"
    );
}

#[test]
fn a_scroll_container_bounded_by_a_maximum_moves_whole_to_the_next_column() {
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:auto;max-height:60px'>X<br>Y<br>Z<br>W<br>V<br>U<br>T</div>"
        )),
        "s=100,0,100,60 X@100,0"
    );
    // Below its maximum, it is as tall as its content, which fits.
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:hidden;max-height:60px'>X</div>\
             <div id=b style='height:10px'></div>"
        )),
        "s=0,80,100,10 b=0,90,100,10 X@0,80"
    );
}

#[test]
fn a_scroll_container_sized_by_its_ratio_moves_to_the_next_column() {
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:auto;width:50px;aspect-ratio:1'>X{LARGE}</div>"
        )),
        "s=100,0,50,50 X@100,0"
    );
}

#[test]
fn absolutely_positioned_boxes_a_scroll_container_contains_move_with_it() {
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:auto;position:relative;height:60px'>X{LARGE}\
             <div id=p style='position:absolute;top:20px;width:10px;height:10px'></div></div>"
        )),
        "s=100,0,100,60 p=100,20,10,10 X@100,0"
    );
    // The containing block may also be a box inside the scroll container.
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:auto;height:60px'><div style='position:relative'>X\
             <div id=p style='position:absolute;top:5px;width:10px;height:10px'></div></div></div>"
        )),
        "s=100,0,100,60 p=100,5,10,10 X@100,0"
    );
}

#[test]
fn a_sticky_scroll_container_moves_to_the_next_column() {
    assert_eq!(
        filled(&format!(
            "{TALL}<div id=s style='overflow:auto;position:sticky;top:0;height:60px'>X{LARGE}</div>"
        )),
        "s=100,0,100,60 X@100,0"
    );
}
