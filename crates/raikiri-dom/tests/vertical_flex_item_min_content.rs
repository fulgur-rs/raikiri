//! A vertical item of a flex container whose `inline-size`, or physical
//! `height`, is `min-content` is as tall as its longest word and has as many
//! lines across as that height gives (CSS Sizing 3 section 3.1.1). Taffy sees
//! the height as `auto`, so a flex row measured the width of the item with the
//! lines in the room of the container, one line for an open container, and
//! left the item a column wide whose height was then the keyword's. The
//! expected border boxes are the ones Chromium lays out for the same markup,
//! in an 800px by 600px viewport without margins and in Ahem at 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A row of the table: the markup of the page body and the boxes
/// `id:x,y,width,height` of the ids that Chromium lays out for it.
type Row = (&'static str, &'static str);

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Check that every row of `rows` is laid out as Chromium lays it out.
fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let actual = boxes(body);
        for entry in expected.split(' ') {
            let (id, rect) = entry.split_once(':').expect("an id and a box");
            let want: Vec<f32> = rect
                .split(',')
                .map(|n| n.parse().expect("a number"))
                .collect();
            let got = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
                .1;
            assert!(
                got.iter()
                    .zip(&want)
                    .all(|(got, want)| (got - want).abs() < 0.01),
                "#{id} is {got:?}, expected {want:?} for {body}"
            );
        }
    }
}

/// The rows where the paragraph of a vertical item is laid out at its `min-content` height.
const MIN_CONTENT_ROWS: &[Row] = &[
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,44,50 b:44,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,54,70 b:54,0,10,70",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,30,40 b:38,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,40,60 b:48,0,10,68",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;height:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,44,50 b:44,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,54,70 b:54,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,30,40 b:38,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,40,60 b:48,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;height:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,100",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;height:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,40 b:0,40,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,60 b:0,60,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,40 b:0,40,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,60 b:0,60,800,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,44,50 b:44,0,10,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,54,70 b:54,0,10,70",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,30,40 b:38,0,10,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,40,60 b:48,0,10,68",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;height:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,30,40 b:30,0,10,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:4,4,40,60 b:48,0,10,68",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:sideways-lr;height:min-content;min-block-size:50px">xxxx xxxx xx</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,50,40 b:50,0,10,100",
    ),
    (
        r#"<div style="display:flex;width:200px;gap:7px"><div id=a style="writing-mode:sideways-rl;inline-size:min-content;box-sizing:content-box;padding:4px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,51,68 b:58,0,10,68",
    ),
    (
        r#"<div style="display:flex;width:200px;gap:7px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,0,40,60 b:47,0,10,60",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:150px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,0,40,60",
    ),
    (
        r#"<div style="display:flex;width:200px;gap:7px"><div id=a style="writing-mode:vertical-rl;height:min-content;flex-grow:1">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:0,0,183,60 b:190,0,10,60",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:vertical-lr;inline-size:min-content;align-self:flex-end">xxxx xxxx xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,60,30,40 b:30,25,10,50",
    ),
    (
        r#"<div style="display:flex;width:60px"><div id=a style="writing-mode:sideways-lr;height:min-content;width:25px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div></div>"#,
        "a:0,0,25,60",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:vertical-lr;height:min-content;align-self:center">xx xx xx xx xx xx</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,40,60,20 b:60,25,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:sideways-lr;inline-size:min-content;box-sizing:content-box;padding:4px">xx xx xx xx xx xx</div></div>"#,
        "a:0,0,68,28",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:vertical-lr;inline-size:min-content;justify-self:end">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,0,40,60 b:40,0,10,60",
    ),
    (
        r#"<div style="display:flex;width:60px"><div id=a style="writing-mode:vertical-lr;height:min-content;">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,0,40,60",
    ),
    (
        r#"<div style="display:flex;align-items:flex-end;height:80px"><div id=a style="writing-mode:sideways-rl;height:min-content;line-height:15px">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,20,60,60",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:vertical-lr;inline-size:min-content;box-sizing:content-box;padding:4px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,16,48,68 b:48,25,10,50",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;align-self:flex-end">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div></div>"#,
        "a:0,40,40,60",
    ),
    (
        r#"<div style="display:flex;align-items:flex-end;height:80px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;box-sizing:content-box;padding:4px">xx xxxxxx xxxx xx</div></div>"#,
        "a:0,12,48,68",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:sideways-rl;inline-size:min-content;line-height:15px">xx xx xx xx xx xx</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,90,20 b:90,0,10,50",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:sideways-lr;height:min-content;">xx xxxxxx xxxx xx</div></div>"#,
        "a:0,0,40,60",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:sideways-rl;height:min-content;box-sizing:content-box;padding:4px">xx xx xx xx xx xx</div></div>"#,
        "a:0,0,68,28",
    ),
    (
        r#"<div style="display:flex;width:60px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;line-height:15px">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,0,60,60",
    ),
    (
        r#"<div style="display:flex;width:60px"><div id=a style="writing-mode:sideways-rl;inline-size:min-content;border:2px solid">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,0,44,64 b:44,0,10,64",
    ),
    (
        r#"<div style='display:flex;flex-direction:column;width:100px'><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,100,60 b:0,60,10,50",
    ),
    (
        r#"<div style='display:flex;flex-direction:column;width:100px'><div id=a style="writing-mode:vertical-lr;height:min-content">xx xxxxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,100,60 b:0,60,10,50",
    ),
];

/// Rows that already matched before the change and must keep matching.
const UNCHANGED_ROWS: &[Row] = &[
    (
        r#"<div style="display:grid;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,60 b:0,60,800,40",
    ),
    (
        r#"<div style="display:grid;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:none">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,40 b:0,45,800,55",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,60 b:0,60,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;padding:3px 5px;border:2px solid">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,70 b:0,70,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;min-inline-size:max-content">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,170 b:0,170,800,50",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:max-content;max-inline-size:50px">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,50 b:0,50,800,50",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:200px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,190,40 b:190,0,10,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,790,40 b:790,0,10,50",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,800,40 b:0,40,800,50",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;max-inline-size:max-content">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,20,100 b:20,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;flex:1 1 0">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:0,0,790,40 b:790,0,10,100",
    ),
    (
        r#"<div style="display:grid;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,792,40 b:0,49,800,51",
    ),
    (
        r#"<div style="display:grid;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;margin:4px">xx xxxxxx xxxx xx</div><div id=b style="writing-mode:vertical-rl;">yy yy</div></div>"#,
        "a:4,4,792,60 b:0,68,800,32",
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;width:150px"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(25px);flex-shrink:0">x</div></div>"#,
        "a:0,0,10,10",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(25px);flex-grow:1">xxxx xxxx xx</div></div>"#,
        "a:0,0,800,120",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:sideways-rl;inline-size:fit-content;margin:4px">xxxx xxxx xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:4,4,792,120 b:0,128,800,50",
    ),
    (
        r#"<div style="display:flex;align-items:flex-start"><div id=a style="writing-mode:sideways-lr;inline-size:min-content;align-self:center">xxxxxxxxxx</div></div>"#,
        "a:0,0,10,100",
    ),
    (
        r#"<div style="display:flex;align-items:flex-start"><div id=a style="writing-mode:vertical-rl;inline-size:30px;padding:3px 5px">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:0,0,50,30 b:50,0,10,50",
    ),
    (
        r#"<div style="display:grid;grid-auto-flow:column"><div id=a style="writing-mode:vertical-rl;height:max-content;">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div></div>"#,
        "a:0,0,800,190",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:sideways-rl;inline-size:fit-content(60px);justify-self:end">xx xx xx xx xx xx</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:790,0,10,170 b:0,170,800,50",
    ),
    (
        r#"<div style="display:grid;justify-items:start"><div id=a style="writing-mode:vertical-lr;inline-size:max-content;justify-self:end">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div></div>"#,
        "a:790,0,10,190",
    ),
    (
        r#"<div style="display:flex;flex-direction:row-reverse"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(60px);flex-shrink:0">xx xx xx xx xx xx</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:790,0,10,170 b:780,0,10,170",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:sideways-lr;height:auto;flex:1 1 0">xxxxxxxxxx</div></div>"#,
        "a:0,0,800,100",
    ),
    (
        r#"<div style="display:flex;align-items:flex-end;height:80px"><div id=a style="writing-mode:vertical-rl;inline-size:min-content;max-block-size:20px">xx xxxxxx xxxx xx</div></div>"#,
        "a:0,20,20,60",
    ),
    (
        r#"<div style="display:grid;grid-template-columns:auto auto;gap:5px"><div id=a style="writing-mode:sideways-lr;height:max-content;width:25px">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,0,25,90",
    ),
    (
        r#"<div style="display:grid;align-items:start;height:90px"><div id=a style="writing-mode:vertical-lr;height:30px;">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:0,0,800,30",
    ),
    (
        r#"<div style="display:flex;flex-direction:row-reverse"><div id=a style="writing-mode:vertical-lr;inline-size:max-content;margin:4px">x</div></div>"#,
        "a:786,4,10,10",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content;align-self:flex-end">xxxxxxxxxx</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:0,0,10,100 b:10,25,10,50",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:sideways-lr;inline-size:auto;padding:2px;border:3px solid;margin:1px 2px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:2,1,53,98 b:57,0,10,100",
    ),
    (
        r#"<div style="display:flex;align-items:flex-end;height:80px"><div id=a style="writing-mode:sideways-rl;inline-size:min-content;">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:0,20,40,60 b:40,30,10,50",
    ),
    (
        r#"<div style="display:grid;grid-template-columns:1fr 1fr"><div id=a style="writing-mode:vertical-lr;height:min-content;width:25px">xxxxxxxxxx</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,0,25,100 b:400,0,400,100",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(25px);align-self:center">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:385,0,30,90",
    ),
    (
        r#"<div style="display:flex;align-items:flex-start"><div id=a style="writing-mode:sideways-lr;inline-size:min-content;width:25px">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-lr">yy yy</div></div>"#,
        "a:0,0,25,60 b:25,0,10,50",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:sideways-lr;inline-size:auto;padding:2px;border:3px solid;margin:1px 2px">xx xx xx xx xx xx</div></div>"#,
        "a:2,1,30,98",
    ),
    (
        r#"<div style="display:flex;flex-direction:row-reverse"><div id=a style="writing-mode:vertical-rl;height:fit-content(25px);max-block-size:20px">xx xx xx xx xx xx</div></div>"#,
        "a:790,0,10,170",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:sideways-rl;inline-size:30px;border:2px solid">xxxx xxxx xx</div></div>"#,
        "a:0,35,34,30",
    ),
    (
        r#"<div style="display:flex;height:100px;align-items:center"><div id=a style="writing-mode:sideways-lr;height:30px;min-block-size:50px">xx xx xx xx xx xx</div></div>"#,
        "a:0,35,60,30",
    ),
    (
        r#"<div style="display:grid;grid-auto-flow:column"><div id=a style="writing-mode:sideways-lr;inline-size:fit-content(25px);">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,401.5,190 b:401.5,0,398.5,190",
    ),
    (
        r#"<div style="display:grid;grid-auto-flow:column"><div id=a style="writing-mode:vertical-rl;inline-size:auto;margin:4px">xx xxxxxx <span style="font-size:20px">xxx</span> xx</div></div>"#,
        "a:4,4,792,190",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:sideways-lr;inline-size:auto;padding:2px;border:3px solid;margin:1px 2px">xxxx<br>xx xxxxxx<br>x</div></div>"#,
        "a:2,1,40,100",
    ),
    (
        r#"<div style="display:inline-flex"><div id=a style="writing-mode:vertical-lr;inline-size:min-content;flex:1 1 0">xxxxxxxxxx</div></div>"#,
        "a:0,0,10,100",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:sideways-lr;inline-size:fit-content(60px);min-block-size:50px">x</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,50,100 b:50,0,10,100",
    ),
    (
        r#"<div style="display:grid;justify-items:start"><div id=a style="writing-mode:vertical-rl;inline-size:fit-content;max-block-size:20px">xxxxxxxxxx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,0,10,100 b:0,100,10,50",
    ),
    (
        r#"<div style="display:flex;height:100px"><div id=a style="writing-mode:vertical-rl;inline-size:fit-content(60px);justify-self:end">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,30,100 b:30,0,10,100",
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div id=a style="writing-mode:vertical-rl;inline-size:fit-content(25px);block-size:35px">xx xx xx xx xx xx</div></div>"#,
        "a:0,0,35,170",
    ),
    (
        r#"<div style="display:grid;justify-items:start"><div id=a style="writing-mode:sideways-lr;inline-size:max-content;align-self:flex-end">x</div></div>"#,
        "a:0,0,10,10",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:vertical-lr;inline-size:min-content;flex:1 1 0">xxxx xxxx xx</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:0,0,800,40 b:0,40,800,50",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:sideways-rl;inline-size:fit-content;">x</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,0,800,10 b:0,10,800,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:sideways-rl;inline-size:fit-content(25px);width:25px">x</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,25,50 b:25,0,10,50",
    ),
    (
        r#"<div style="display:grid"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(25px);">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,0,800,90 b:0,90,800,50",
    ),
    (
        r#"<div style="display:flex"><div id=a style="writing-mode:vertical-lr;inline-size:fit-content(60px);box-sizing:content-box;padding:4px">xxxxxxxxxx</div><div id=b style="writing-mode:sideways-lr">yy yy</div></div>"#,
        "a:0,0,18,108 b:18,0,10,108",
    ),
    (
        r#"<div style="display:grid;grid-template-columns:1fr 1fr"><div id=a style="writing-mode:sideways-rl;height:min-content;margin:4px">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:sideways-rl">yy yy</div></div>"#,
        "a:4,4,392,60 b:400,0,400,68",
    ),
    (
        r#"<div style="display:flex;width:60px"><div id=a style="writing-mode:sideways-rl;inline-size:30px;">x</div></div>"#,
        "a:0,0,10,30",
    ),
    (
        r#"<div style="display:grid;height:100px"><div id=a style="writing-mode:sideways-rl;inline-size:max-content;max-block-size:20px">xx xx xx xx xx xx</div></div>"#,
        "a:0,0,20,170",
    ),
    (
        r#"<div style="display:grid;grid-template-columns:1fr 1fr"><div id=a style="writing-mode:sideways-lr;inline-size:fit-content(60px);width:25px">x</div></div>"#,
        "a:0,0,25,10",
    ),
    (
        r#"<div style="display:flex;width:200px;gap:7px"><div id=a style="writing-mode:sideways-rl;inline-size:30px;flex:1 1 0">x</div></div>"#,
        "a:0,0,200,30",
    ),
    (
        r#"<div style="display:grid;grid-auto-flow:column"><div id=a style="writing-mode:sideways-lr;inline-size:min-content;align-self:flex-end">xxxx<br>xx xxxxxx<br>x</div><div id=b style="writing-mode:vertical-rl">yy yy</div></div>"#,
        "a:0,0,415,60 b:415,0,385,60",
    ),
    (
        r#"<div style='writing-mode:vertical-lr;display:flex;height:100px'><div id=a style="writing-mode:vertical-lr;inline-size:max-content">xx xxxxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,20,100 b:0,100,10,50",
    ),
    (
        r#"<div style='writing-mode:vertical-lr;display:flex;height:100px'><div id=a style="writing-mode:vertical-rl;max-height:min-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:0,100,10,50",
    ),
    (
        r#"<div id=a style="writing-mode:vertical-lr;max-height:min-content">xxxxxxxxxx</div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:0,100,10,50",
    ),
    (
        r#"<div style='height:50px'><div id=a style="writing-mode:vertical-lr;inline-size:fit-content">xx xxxxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,40,60 b:0,50,10,50",
    ),
    (
        r#"<div style='height:50px'><div id=a style="writing-mode:vertical-rl;min-height:min-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:0,50,10,50",
    ),
    (
        r#"<div id=a style="writing-mode:vertical-lr;height:min-content">xxxxxxxxxx</div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:0,100,10,50",
    ),
    (
        r#"<div style='float:left'><div id=a style="writing-mode:vertical-lr;inline-size:fit-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:10,0,10,50",
    ),
    (
        r#"<div style='position:absolute'><div id=a style="writing-mode:vertical-rl;inline-size:max-content">xxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,120 b:0,0,10,50",
    ),
    (
        r#"<div style='display:grid;grid-template-columns:100px 100px'><div id=a style="writing-mode:vertical-lr;inline-size:min-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,100,100 b:0,100,10,50",
    ),
    (
        r#"<div style='display:grid;grid-template-columns:100px 100px'><div id=a style="writing-mode:vertical-rl;max-height:min-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,100,100 b:0,100,10,50",
    ),
    (
        r#"<div style='display:table'><div style='display:table-row'><div style='display:table-cell'><div id=a style="writing-mode:vertical-rl;height:min-content">xx xxxxxx xxxx xx</div></div></div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,40,60 b:0,60,10,50",
    ),
    (
        r#"<div style='columns:2;width:200px'><div id=a style="writing-mode:vertical-rl;inline-size:min-content">xxxx xxxx xx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,30,40 b:0,40,10,50",
    ),
    (
        r#"<div style='columns:2;width:200px'><div id=a style="writing-mode:vertical-rl;height:min-content">xxxxxxxxxx</div></div><div id=b style="writing-mode:vertical-rl">yy yy</div>"#,
        "a:0,0,10,100 b:0,100,10,50",
    ),
];

#[test]
fn a_min_content_inline_size_breaks_the_lines_of_a_vertical_flex_item_at_the_longest_word() {
    assert_rows(MIN_CONTENT_ROWS);
}

#[test]
fn the_other_keywords_and_the_other_containers_keep_their_boxes() {
    assert_rows(UNCHANGED_ROWS);
}
