//! `align-content` moves the content of a block container that holds only
//! inline content, as it moves block children (CSS Box Alignment 3 §5.1): the
//! lines, the atomic inlines and floats on them, block boxes between them, the
//! static position of an absolutely positioned box, and the baseline the box
//! exports. Overflowing content stays at the start edge unless the alignment
//! is `unsafe`. The positions are the ones Chromium gives for the same
//! documents, with Ahem at 20px on 20px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The offset of the box `#t` from the box `#r` in `body`, and its height.
fn placed(body: &str) -> (f32, f32, f32) {
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
         body{{margin:0;font:20px/20px Ahem}}</style><body>{body}</body>"
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
    let page = result.pages().next().unwrap();
    let dom = page.dom();
    let (mut root, mut target) = (None, None);
    for fragment in page.fragments() {
        match dom.attr(fragment.node(), "id") {
            Some("r") => root = Some(fragment.rect()),
            Some("t") => target = Some(fragment.rect()),
            _ => {}
        }
    }
    let (root, target) = (root.unwrap(), target.unwrap());
    (target.x - root.x, target.y - root.y, target.height)
}

#[test]
fn the_lines_of_a_block_move_to_the_alignment_it_states() {
    for (alignment, top) in [
        ("start", 0.0),
        ("center", 20.0),
        ("end", 40.0),
        ("space-between", 0.0),
        ("space-around", 20.0),
    ] {
        assert_eq!(
            placed(&format!(
                "<div id=r style='height:60px;align-content:{alignment}'><span id=t>X</span></div>"
            )),
            (0.0, top, 20.0),
            "{alignment}"
        );
    }
    assert_eq!(
        placed(
            "<div id=r style='height:60px;width:20px;align-content:center'>\
             <span id=t>X X</span></div>"
        ),
        (0.0, 10.0, 40.0)
    );
    assert_eq!(
        placed(
            "<div id=r style='height:60px;padding:5px 0 15px;align-content:center'>\
             <span id=t>X</span></div>"
        ),
        (0.0, 25.0, 20.0)
    );
}

#[test]
fn overflowing_lines_stay_at_the_start_unless_the_alignment_is_unsafe() {
    assert_eq!(
        placed("<div id=r style='height:10px;align-content:end'><span id=t>X</span></div>"),
        (0.0, 0.0, 20.0)
    );
    assert_eq!(
        placed(
            "<div style='height:50px'></div>\
             <div id=r style='height:10px;align-content:unsafe end'><span id=t>X</span></div>"
        ),
        (0.0, -10.0, 20.0)
    );
}

#[test]
fn the_boxes_on_and_between_the_lines_move_with_them() {
    let inline_block = "<span id=t style='display:inline-block;width:10px;height:10px'></span>";
    assert_eq!(
        placed(&format!(
            "<div id=r style='height:60px;align-content:end'>X{inline_block}</div>"
        )),
        (20.0, 46.0, 10.0)
    );
    assert_eq!(
        placed(
            "<div id=r style='height:60px;align-content:end'>X<span id=t \
             style='position:relative;top:5px;display:inline-block;width:10px;height:10px'>\
             </span></div>"
        ),
        (20.0, 51.0, 10.0)
    );
    assert_eq!(
        placed(
            "<div id=r style='height:60px;align-content:end'>\
             <div id=t style='float:left;width:10px;height:10px'></div>X</div>"
        ),
        (0.0, 40.0, 10.0)
    );
    assert_eq!(
        placed(
            "<div id=r style='height:100px;align-content:end'>X\
             <div id=t style='height:10px'></div>Y</div>"
        ),
        (0.0, 70.0, 10.0)
    );
    assert_eq!(
        placed(
            "<div id=r style='height:60px;align-content:end;position:relative'>X\
             <span id=t style='position:absolute;width:10px;height:10px'></span></div>"
        ),
        (20.0, 40.0, 10.0)
    );
}

#[test]
fn a_float_taller_than_the_lines_is_part_of_the_aligned_content() {
    assert_eq!(
        placed(
            "<div id=r style='height:100px;align-content:end'>\
             <div id=t style='float:left;width:10px;height:80px'></div>X</div>"
        ),
        (0.0, 20.0, 80.0)
    );
}

#[test]
fn an_absolutely_positioned_box_after_the_lines_follows_the_aligned_content() {
    let positioned = "<div id=t style='position:absolute;width:10px;height:10px'></div>";
    assert_eq!(
        placed(&format!(
            "<div id=r style='height:100px;align-content:end;position:relative'>X<br>{positioned}</div>"
        )),
        (0.0, 100.0, 10.0)
    );
    assert_eq!(
        placed(&format!(
            "<div id=r style='height:100px;align-content:end;position:relative'>{positioned}</div>"
        )),
        (0.0, 100.0, 10.0)
    );
    // More than one such box is placed from an index of the paragraph.
    for content in ["", "X<br>"] {
        assert_eq!(
            placed(&format!(
                "<div id=r style='height:100px;align-content:end;position:relative'>\
                 {content}{positioned}\
                 <div style='position:absolute;width:10px;height:10px'></div></div>"
            )),
            (0.0, 100.0, 10.0),
            "{content:?}"
        );
    }
}

#[test]
fn the_baseline_of_the_block_moves_with_its_lines() {
    assert_eq!(
        placed(
            "<div style='display:flex;align-items:baseline'>\
             <div id=r style='height:60px;align-content:end'>X</div><span id=t>Y</span></div>"
        ),
        (20.0, 40.0, 20.0)
    );
    // A grid measures the baseline of its items before it lays them out.
    assert_eq!(
        placed(
            "<div style='display:grid;grid-template-columns:20px auto;align-items:baseline'>\
             <div id=r style='height:60px;align-content:end'>X</div><span id=t>Y</span></div>"
        )
        .1,
        40.0
    );
    // An inline-block takes the baseline of the last line inside it.
    assert_eq!(
        placed(
            "<div><span id=r style='display:inline-block'>\
             <div style='height:60px;align-content:end'>X</div></span><span id=t>Y</span></div>"
        ),
        (20.0, 40.0, 20.0)
    );
}
