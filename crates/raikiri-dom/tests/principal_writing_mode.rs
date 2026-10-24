//! The root element takes the used `writing-mode` and `direction` of its
//! `body` child, whatever it declares itself, and keeps its computed values
//! (CSS Writing Modes 4 §8, the principal writing mode). A vertical body
//! under a horizontal root is therefore no orthogonal flow: the root, the
//! viewport and the initial containing block run on the body's axes.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements with an `id`.
type Boxes = Vec<(String, Rect)>;

/// The border boxes of the elements of `html` with an `id`, in the order they
/// were found, one list per page.
fn pages_of(html: &str) -> Vec<Boxes> {
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
    result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter_map(|fragment| {
                    let id = dom.attr(fragment.node(), "id")?;
                    let rect = fragment.rect();
                    Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
                })
                .collect()
        })
        .collect()
}

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Boxes {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    pages_of(&html).swap_remove(0)
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn the_body_writing_mode_is_the_used_writing_mode_of_the_root() {
    check(
        "<html id=h><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"writing-mode:vertical-rl;height:300px\"><div id=d style=\"background:green;width:30px;height:100px\"></div></body>",
        &[
            ("b", (762.0, 8.0, 30.0, 300.0)),
            ("d", (762.0, 8.0, 30.0, 100.0)),
            ("h", (754.0, 0.0, 46.0, 600.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"writing-mode:vertical-lr\"><div id=d>ABC</div><div id=e>DEFGH</div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 10.0, 584.0)),
            ("e", (18.0, 8.0, 10.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"writing-mode:sideways-rl\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (772.0, 8.0, 20.0, 584.0)),
            ("d", (772.0, 8.0, 20.0, 584.0)),
            ("h", (764.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"writing-mode:sideways-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"width:300px;height:100px\"><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 84.0)),
            ("d", (8.0, 8.0, 20.0, 84.0)),
            ("h", (0.0, 0.0, 300.0, 100.0)),
        ],
    );
}

#[test]
fn the_body_writing_mode_replaces_the_one_the_root_declares() {
    check(
        "<html id=h><body id=b style=\"writing-mode:vertical-rl\"><div id=d style=\"background:green;block-size:20px;inline-size:30px\"></div><div id=e style=\"background:red;block-size:20px\"></div></body>",
        &[
            ("b", (752.0, 8.0, 40.0, 584.0)),
            ("d", (772.0, 8.0, 20.0, 30.0)),
            ("e", (752.0, 8.0, 20.0, 584.0)),
            ("h", (744.0, 0.0, 56.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-rl\"><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-lr\"><body id=b style=\"writing-mode:vertical-rl\"><div id=d style=\"background:green;block-size:20px\"></div><div id=e style=\"background:red;block-size:20px\"></div></body>",
        &[
            ("b", (752.0, 8.0, 40.0, 584.0)),
            ("d", (772.0, 8.0, 20.0, 584.0)),
            ("e", (752.0, 8.0, 20.0, 584.0)),
            ("h", (744.0, 0.0, 56.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-lr\"><body id=b style=\"writing-mode:horizontal-tb\"><div id=d style=\"background:green;height:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 784.0, 20.0)),
            ("d", (8.0, 8.0, 784.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-lr\"><body id=b style=\"writing-mode:horizontal-tb;width:300px\"><div id=d style=\"background:green;height:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 300.0, 20.0)),
            ("d", (8.0, 8.0, 300.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-lr\"><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"writing-mode:vertical-lr\"><body id=b><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 36.0, 600.0)),
        ],
    );
}

#[test]
fn the_body_direction_is_the_used_direction_of_the_root() {
    check(
        "<html id=h><body id=b style=\"direction:rtl\"><div id=d style=\"background:green;height:20px;width:100px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 784.0, 20.0)),
            ("d", (692.0, 8.0, 100.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h style=\"direction:rtl\"><body id=b style=\"direction:ltr\"><div id=d style=\"background:green;height:20px;width:100px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 784.0, 20.0)),
            ("d", (8.0, 8.0, 100.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"direction:rtl;width:100px\"><div id=d style=\"background:green;height:20px;width:50px\"></div></body>",
        &[
            ("b", (692.0, 8.0, 100.0, 20.0)),
            ("d", (742.0, 8.0, 50.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h style=\"direction:rtl\"><body id=b style=\"direction:ltr;width:100px\"><div id=d style=\"background:green;height:20px;width:50px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 100.0, 20.0)),
            ("d", (8.0, 8.0, 50.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h style=\"direction:ltr\"><body id=b style=\"direction:rtl;width:100px\"><div id=d style=\"background:green;height:20px;width:50px\"></div></body>",
        &[
            ("b", (692.0, 8.0, 100.0, 20.0)),
            ("d", (742.0, 8.0, 50.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 36.0)),
        ],
    );
    check(
        "<html id=h><body id=b style=\"writing-mode:vertical-rl;direction:rtl\"><div id=d style=\"background:green;block-size:20px;inline-size:50px\"></div></body>",
        &[
            ("b", (772.0, 8.0, 20.0, 584.0)),
            ("d", (772.0, 542.0, 20.0, 50.0)),
            ("h", (764.0, 0.0, 36.0, 600.0)),
        ],
    );
}

#[test]
fn flex_gaps_in_a_vertical_body_lay_out_as_chromium_does() {
    check(
        "<style>body{writing-mode:vertical-lr} section{background-color:green;block-size:100px;display:flex;gap:20px} section>div{background-color:grey;flex:1 1 auto}</style><p>Test passes if there is <strong>a green line between boxes</strong>.</p><section id=sec><div id=a></div><div id=b></div><div id=c></div></section>",
        &[
            ("a", (30.0, 8.0, 100.0, 181.33)),
            ("b", (30.0, 209.33, 100.0, 181.34)),
            ("c", (30.0, 410.67, 100.0, 181.33)),
            ("sec", (30.0, 8.0, 100.0, 584.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-rl} section{background-color:green;block-size:100px;display:flex;gap:20px} section>div{background-color:grey;flex:1 1 auto}</style><p>Test passes if there is <strong>a green line between boxes</strong>.</p><section id=sec><div id=a></div><div id=b></div><div id=c></div></section>",
        &[
            ("a", (670.0, 8.0, 100.0, 181.33)),
            ("b", (670.0, 209.33, 100.0, 181.34)),
            ("c", (670.0, 410.67, 100.0, 181.33)),
            ("sec", (670.0, 8.0, 100.0, 584.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-lr} section{background-color:green;block-size:200px;display:flex;flex-direction:column;gap:20px} section>div{background-color:grey;flex:1 1 auto}</style><p>Test passes if there is <strong>a green vertical line between boxes</strong>.</p><section id=sec><div id=a></div><div id=b></div><div id=c></div></section>",
        &[
            ("a", (40.0, 8.0, 53.33, 584.0)),
            ("b", (113.33, 8.0, 53.34, 584.0)),
            ("c", (186.67, 8.0, 53.33, 584.0)),
            ("sec", (40.0, 8.0, 200.0, 584.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-rl} section{background-color:green;block-size:200px;display:flex;flex-direction:column;gap:20px} section>div{background-color:grey;flex:1 1 auto}</style><p>Test passes if there is <strong>a green vertical line between boxes</strong>.</p><section id=sec><div id=a></div><div id=b></div><div id=c></div></section>",
        &[
            ("a", (706.67, 8.0, 53.33, 584.0)),
            ("b", (633.33, 8.0, 53.34, 584.0)),
            ("c", (560.0, 8.0, 53.33, 584.0)),
            ("sec", (560.0, 8.0, 200.0, 584.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-lr} section{background-color:green;display:inline-flex;flex-direction:column;gap:20px} section>div{background-color:grey;color:white}</style><p>Test passes if there are <strong> green lines between boxes</strong>.</p><section id=sec><div id=a>Black Panther</div><div id=b>Wonder Woman</div><div id=c>Storm</div><div id=d>Flash</div></section>",
        &[
            ("a", (30.0, 8.0, 10.0, 130.0)),
            ("b", (60.0, 8.0, 10.0, 130.0)),
            ("c", (90.0, 8.0, 10.0, 130.0)),
            ("d", (120.0, 8.0, 10.0, 130.0)),
            ("sec", (30.0, 8.0, 100.0, 130.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-rl} section{background-color:green;display:inline-flex;flex-direction:column;gap:20px} section>div{background-color:grey;color:white}</style><p>Test passes if there are <strong> green lines between boxes</strong>.</p><section id=sec><div id=a>Black Panther</div><div id=b>Wonder Woman</div><div id=c>Storm</div><div id=d>Flash</div></section>",
        &[
            ("a", (760.0, 8.0, 10.0, 130.0)),
            ("b", (730.0, 8.0, 10.0, 130.0)),
            ("c", (700.0, 8.0, 10.0, 130.0)),
            ("d", (670.0, 8.0, 10.0, 130.0)),
            ("sec", (670.0, 8.0, 100.0, 130.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-lr} section{background-color:green;block-size:100px;inline-size:200px;display:inline-flex;flex-wrap:wrap;gap:20px} section>div{background-color:grey;color:white;block-size:20px} #a{inline-size:120px} #b{inline-size:130px} #c,#d{inline-size:40px}</style><p>Test passes if there are <strong> green lines between boxes</strong>.</p><section id=sec><div id=a>Black Panther</div><div id=b>Wonder Woman</div><div id=c>Storm</div><div id=d>Flash</div></section>",
        &[
            ("a", (30.0, 8.0, 20.0, 120.0)),
            ("b", (70.0, 8.0, 20.0, 130.0)),
            ("c", (70.0, 158.0, 20.0, 40.0)),
            ("d", (110.0, 8.0, 20.0, 40.0)),
            ("sec", (30.0, 8.0, 100.0, 200.0)),
        ],
    );
    check(
        "<style>body{writing-mode:vertical-rl} section{background-color:green;block-size:100px;inline-size:200px;display:inline-flex;flex-wrap:wrap;gap:20px} section>div{background-color:grey;color:white;block-size:20px} #a{inline-size:120px} #b{inline-size:130px} #c,#d{inline-size:40px}</style><p>Test passes if there are <strong> green lines between boxes</strong>.</p><section id=sec><div id=a>Black Panther</div><div id=b>Wonder Woman</div><div id=c>Storm</div><div id=d>Flash</div></section>",
        &[
            ("a", (750.0, 8.0, 20.0, 120.0)),
            ("b", (710.0, 8.0, 20.0, 130.0)),
            ("c", (710.0, 158.0, 20.0, 40.0)),
            ("d", (670.0, 8.0, 20.0, 40.0)),
            ("sec", (670.0, 8.0, 100.0, 200.0)),
        ],
    );
}

#[test]
fn the_logical_sizes_of_the_root_follow_its_own_writing_mode() {
    check(
        "<html id=h style=\"writing-mode:vertical-lr;inline-size:300px\"><body id=b style=\"writing-mode:horizontal-tb\"><div id=d style=\"background:green;height:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 784.0, 20.0)),
            ("d", (8.0, 8.0, 784.0, 20.0)),
            ("h", (0.0, 0.0, 800.0, 300.0)),
        ],
    );
    check(
        "<html id=h style=\"inline-size:300px\"><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 584.0)),
            ("d", (8.0, 8.0, 20.0, 584.0)),
            ("h", (0.0, 0.0, 300.0, 600.0)),
        ],
    );
    check(
        "<html id=h style=\"block-size:300px\"><body id=b style=\"writing-mode:vertical-lr\"><div id=d style=\"background:green;block-size:20px\"></div></body>",
        &[
            ("b", (8.0, 8.0, 20.0, 284.0)),
            ("d", (8.0, 8.0, 20.0, 284.0)),
            ("h", (0.0, 0.0, 36.0, 300.0)),
        ],
    );
}
