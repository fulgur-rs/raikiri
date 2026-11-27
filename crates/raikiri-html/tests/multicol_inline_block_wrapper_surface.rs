//! Blocks inside an inline box in a multicol container. The inline box is
//! split around the blocks it holds (CSS 2, 9.2.1.1), so when it draws
//! nothing of its own the blocks go in the columns as if the container held
//! them: they balance across the columns, fill definite-height columns in
//! turn, and a spanner among them spans the columns. The positions are the
//! ones Chromium gives the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

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

/// The fragments of the boxes with an id in `body`, as `id=x,y,width,height`,
/// sorted by id and then in fragment order.
fn boxes(body: &str) -> String {
    let result = lay_out(body);
    let page = result.page(0).expect("one page");
    assert!(result.page(1).is_none(), "the content fits one page");
    let dom = page.dom();
    let mut out: Vec<(String, String)> = page
        .fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                format!("{id}={},{},{},{}", rect.x, rect.y, rect.width, rect.height),
            ))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A container of two balanced columns 150px wide holding `content`.
fn balanced(content: &str) -> String {
    boxes(&format!(
        "<article style='column-count:2;width:300px;column-gap:0'>{content}</article>"
    ))
}

/// Two 20px blocks with the ids `a` and `b`.
const BLOCKS: &str = "<div id=a style='height:20px'></div><div id=b style='height:20px'></div>";

#[test]
fn the_blocks_of_an_inline_box_balance_across_the_columns() {
    let expected = "a=0,0,150,20 b=150,0,150,20";
    assert_eq!(balanced(&format!("<span>{BLOCKS}</span>")), expected);
    // A width or height does not apply to an inline box.
    assert_eq!(
        balanced(&format!(
            "<span style='width:10px;height:10px'>{BLOCKS}</span>"
        )),
        expected
    );
    // Nor do plain wrappers around it change anything.
    assert_eq!(
        balanced(&format!("<div><em><span>{BLOCKS}</span></em></div>")),
        expected
    );
    // The blocks may hold lines, and the inline box may set inherited text
    // properties that its blocks take.
    assert_eq!(
        balanced("<span style='color:red'><div id=a>A</div><div id=b>B</div></span>"),
        "a=0,0,150,10 b=150,0,150,10"
    );
}

#[test]
fn the_blocks_of_an_inline_box_flow_with_the_blocks_around_it() {
    assert_eq!(
        balanced(&format!(
            "<div id=c style='height:20px'></div><span>{BLOCKS}</span>\
             <div id=d style='height:20px'></div>"
        )),
        "a=0,20,150,20 b=150,0,150,20 c=0,0,150,20 d=150,20,150,20"
    );
}

#[test]
fn the_blocks_of_an_inline_box_fill_definite_height_columns_in_turn() {
    assert_eq!(
        boxes(&format!(
            "<article style='column-count:2;width:300px;column-gap:0;height:30px;\
             column-fill:auto'><span>{BLOCKS}</span></article>"
        )),
        "a=0,0,150,20 b=0,20,150,10 b=150,0,150,10"
    );
}

#[test]
fn a_spanner_in_an_inline_box_inside_a_block_wrapper_spans_the_columns() {
    assert_eq!(
        balanced(
            "<div><span><div id=a style='height:20px'></div>\
             <h2 id=s style='column-span:all;height:10px;margin:0'></h2>\
             <div id=b style='height:20px'></div><div id=c style='height:20px'></div></span></div>"
        ),
        "a=0,0,150,10 a=150,0,150,10 b=0,20,150,20 c=150,20,150,20 s=0,10,300,10"
    );
}
