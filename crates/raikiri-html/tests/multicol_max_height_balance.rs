//! An auto-height multi-column container that only a maximum height bounds
//! balances its columns as one without it does: the maximum is an upper
//! bound on the balanced columns (CSS Multi-column 1 §7.1, §8.1), so the
//! container is as tall as they are, up to it. Content that needs more than
//! the maximum fills the columns at that height and goes on in overflow
//! columns. The heights and positions are the ones Chromium 153 gives for the
//! same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

const ABC: &str = "<div id=a>a</div><div id=b>b</div><div id=c>c</div>";

/// The border box `(x, y, width, height)` of the first fragment of the
/// element with `id` on the first page.
fn rect(body: &str, id: &str) -> (f32, f32, f32, f32) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}<div id=n>n</div>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let document = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("layout must complete")
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let r = fragment.rect();
            (r.x, r.y, r.width, r.height)
        })
        .unwrap_or_else(|| panic!("no fragment for #{id}"))
}

/// The container `m` of two columns 50px wide around `content`, with
/// `style` added.
fn container(style: &str, content: &str) -> String {
    format!("<div id=m style='width:100px;columns:2;column-gap:0;{style}'>{content}</div>")
}

/// The height of the container `m` in `body` and the block offset of the
/// line after it.
fn height(body: &str) -> (f32, f32) {
    (rect(body, "m").3, rect(body, "n").1)
}

#[test]
fn a_maximum_height_bounds_the_balanced_columns() {
    // Three lines balance into columns 20px tall, as they do without the
    // maximum height.
    let body = container("max-height:100px", ABC);
    assert_eq!(height(&body), (20.0, 20.0));
    assert_eq!(rect(&body, "c"), (50.0, 0.0, 50.0, 10.0));
    assert_eq!(
        height(&container("max-block-size:100px", ABC)),
        (20.0, 20.0)
    );
    assert_eq!(height(&container("max-height:20px", ABC)), (20.0, 20.0));
    let five = "<div>a</div><div>b</div><div>c</div><div>d</div><div id=e>e</div>";
    let body = container("max-height:100px;columns:3", five);
    assert_eq!(height(&body), (20.0, 20.0));
    assert_eq!(rect(&body, "e").0, 200.0 / 3.0);
    let nine: String = (0..9).map(|_| "<div>x</div>").collect();
    assert_eq!(height(&container("max-height:100px", &nine)), (50.0, 50.0));
}

#[test]
fn content_taller_than_the_maximum_fills_the_columns_at_it() {
    let body = container("max-height:15px", ABC);
    assert_eq!(height(&body), (15.0, 15.0));
    assert_eq!(rect(&body, "b"), (50.0, 0.0, 50.0, 10.0));
    assert_eq!(rect(&body, "c"), (100.0, 0.0, 50.0, 10.0));
    let nine: String = (0..9).map(|_| "<div>x</div>").collect();
    assert_eq!(height(&container("max-height:30px", &nine)), (30.0, 30.0));
    let float = "<div style='float:left;width:10px;height:10px'></div>";
    assert_eq!(
        height(&container("max-height:15px", &format!("{float}{ABC}"))),
        (15.0, 15.0)
    );
}

#[test]
fn padding_borders_and_box_sizing_add_to_the_balanced_columns() {
    assert_eq!(
        height(&container(
            "max-height:100px;padding:3px;border:2px solid",
            ABC
        )),
        (30.0, 30.0)
    );
    assert_eq!(
        height(&container(
            "max-height:30px;box-sizing:border-box;padding:5px 0",
            ABC
        )),
        (30.0, 30.0)
    );
    // The maximum border box leaves 10px for the columns.
    assert_eq!(
        height(&container(
            "max-height:20px;box-sizing:border-box;padding:5px 0",
            ABC
        )),
        (20.0, 20.0)
    );
}

#[test]
fn a_minimum_and_a_height_still_size_the_container() {
    assert_eq!(
        height(&container("max-height:100px;min-height:40px", ABC)),
        (40.0, 40.0)
    );
    assert_eq!(
        height(&container("height:50px;max-height:100px", ABC)),
        (50.0, 50.0)
    );
}

#[test]
fn columns_filled_in_turn_keep_their_content_height() {
    let body = container("max-height:100px;column-fill:auto", ABC);
    assert_eq!(height(&body), (30.0, 30.0));
    assert_eq!(rect(&body, "c"), (0.0, 20.0, 50.0, 10.0));
    assert_eq!(
        height(&container("max-height:15px;column-fill:auto", ABC)),
        (15.0, 15.0)
    );
}

#[test]
fn a_percentage_of_an_indefinite_height_bounds_nothing() {
    let body = container("max-height:50%", ABC);
    assert_eq!(height(&body), (20.0, 20.0));
    assert_eq!(rect(&body, "b"), (0.0, 10.0, 50.0, 10.0));
    assert_eq!(
        height(&container("max-height:50%;min-height:5px", ABC)),
        (20.0, 20.0)
    );
    // Against a definite height, it bounds the balanced columns.
    let body = format!(
        "<div style='height:200px'>{}</div>",
        container("max-height:50%", ABC)
    );
    assert_eq!(height(&body), (20.0, 200.0));
}

#[test]
fn margins_floats_spanners_and_positioned_boxes_balance_as_without_a_maximum() {
    let margins = "<div style='margin-bottom:10px'>a</div><div>b</div>";
    assert_eq!(
        height(&container("max-height:100px", margins)),
        (15.0, 15.0)
    );
    let float = "<div style='float:left;width:10px;height:10px'></div>";
    assert_eq!(
        height(&container("max-height:100px", &format!("{float}{ABC}"))),
        (20.0, 20.0)
    );
    let spanner = "<div>a</div><div>b</div><div style='column-span:all'>s</div>\
                   <div>c</div><div>d</div>";
    assert_eq!(
        height(&container("max-height:100px", spanner)),
        (30.0, 30.0)
    );
    let positioned =
        format!("{ABC}<div style='position:absolute;top:0;height:50px;width:5px'></div>");
    assert_eq!(
        height(&container(
            "max-height:100px;position:relative",
            &positioned
        )),
        (20.0, 20.0)
    );
    let wrapped = format!("<div>{ABC}</div>");
    assert_eq!(
        height(&container("max-height:100px", &wrapped)),
        (20.0, 20.0)
    );
    let lineless = "<div style='height:10px'></div><div style='height:10px'></div>\
                    <div style='height:10px'></div>";
    assert_eq!(
        height(&container("max-height:100px", lineless)),
        (15.0, 15.0)
    );
}
