//! `stretch` (and `-webkit-fill-available`) as the `min-width`, `max-width`,
//! `min-height` or `max-height` of a form control, and the other atomic
//! inlines that are not replaced elements (a MathML root). A control with the
//! default `display: inline` is an atomic inline that browsers size like an
//! inline-block, so the limits clamp it: the inline axis fills the width of the
//! containing block less the margins, the block axis fills the definite height
//! of the parent less the margins the parent holds in (CSS Sizing 4 §5.1).
//!
//! The expected border boxes are the ones Chromium computes for the same markup
//! in an 800px by 600px viewport, with no body margin and `10px Ahem` text. A
//! control gets no padding or border unless a case says so, and an `input`, a
//! `textarea`, a `select` and a `button` fit every row alike.

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

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }}          html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}          div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= TOLERANCE
}

/// The border box of `#a` in `body`.
fn rect_of_a(body: &str) -> Rect {
    border_boxes(body)
        .into_iter()
        .find(|(name, _)| name == "a")
        .unwrap_or_else(|| panic!("no fragment for #a in {body}"))
        .1
}

/// The markup around a control.
#[derive(Clone, Copy)]
enum Parent {
    /// A 200px wide block whose height follows its content.
    AutoHeight,
    /// A 200px by 100px block.
    FixedHeight,
    /// A 200px wide flex container.
    Flex,
    /// A 200px wide grid container.
    Grid,
    /// A 200px wide block with a word of text before the control.
    Line,
    /// A block that is half the width of the 800px viewport.
    Percent,
    /// A 200px wide block with 10px of padding on each side.
    Padded,
    /// A 200px wide block whose direction is right to left.
    RightToLeft,
}

fn wrap(parent: Parent, control: &str) -> String {
    match parent {
        Parent::AutoHeight => format!("<div style=\"width:200px\">{control}</div>"),
        Parent::FixedHeight => format!("<div style=\"width:200px;height:100px\">{control}</div>"),
        Parent::Flex => format!("<div style=\"display:flex;width:200px\">{control}</div>"),
        Parent::Grid => format!("<div style=\"display:grid;width:200px\">{control}</div>"),
        Parent::Line => format!("<div style=\"width:200px\">abc{control}</div>"),
        Parent::Percent => format!("<div style=\"width:50%\">{control}</div>"),
        Parent::Padded => format!(
            "<div style=\"width:200px;padding:0 10px;box-sizing:content-box\">{control}</div>"
        ),
        Parent::RightToLeft => {
            format!("<div style=\"width:200px;direction:rtl\">{control}</div>")
        }
    }
}

/// The edges of a control: none, or a border and padding added to the
/// content box.
#[derive(Clone, Copy)]
enum Edges {
    /// No border or padding, sized by the border box.
    Plain,
    /// A 1px border, sized by the content box (the default of `input`).
    ContentBox,
}

const CONTROLS: [&str; 4] = ["input", "textarea", "select", "button"];

fn control(tag: &str, edges: Edges, style: &str) -> String {
    let edges = match edges {
        Edges::Plain => "border:0;padding:0;box-sizing:border-box;",
        Edges::ContentBox => "box-sizing:content-box;border:1px solid;",
    };
    if tag == "input" {
        format!("<input id=a style=\"{edges}{style}\">")
    } else {
        format!("<{tag} id=a style=\"{edges}{style}\"></{tag}>")
    }
}

/// One row: a control with `style` in `parent`, and its border box.
struct Case {
    name: &'static str,
    parent: Parent,
    edges: Edges,
    style: &'static str,
    expected: Rect,
}

const fn case(
    name: &'static str,
    parent: Parent,
    edges: Edges,
    style: &'static str,
    expected: Rect,
) -> Case {
    Case {
        name,
        parent,
        edges,
        style,
        expected,
    }
}

fn check(cases: &[Case]) {
    for case in cases {
        for tag in CONTROLS {
            let body = wrap(case.parent, &control(tag, case.edges, case.style));
            let found = rect_of_a(&body);
            let (x, y, width, height) = case.expected;
            assert!(
                close(found.0, x)
                    && close(found.1, y)
                    && close(found.2, width)
                    && close(found.3, height),
                "{}: <{tag}> in {body} is {found:?}, expected {:?}",
                case.name,
                case.expected
            );
        }
    }
}

use Edges::{ContentBox, Plain};
use Parent::{AutoHeight, FixedHeight, Flex, Grid, Line, Padded, Percent, RightToLeft};

/// A control in a 200px wide block, with the width limited by `stretch`.
#[test]
fn a_the_inline_axis_fills_the_containing_block() {
    check(&[
        case(
            "a_min",
            AutoHeight,
            Plain,
            "min-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "a_max",
            AutoHeight,
            Plain,
            "max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "a_fill_available",
            AutoHeight,
            Plain,
            "max-width:-webkit-fill-available;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "a_min_inline_size",
            AutoHeight,
            Plain,
            "min-inline-size:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "a_max_inline_size",
            AutoHeight,
            Plain,
            "max-inline-size:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        // The margins come off the room.
        case(
            "a_min_margins",
            AutoHeight,
            Plain,
            "min-width:stretch;width:50px;height:20px;margin:0 10px 0 5px",
            (5.0, 0.0, 185.0, 20.0),
        ),
        case(
            "a_max_margins",
            AutoHeight,
            Plain,
            "max-width:stretch;width:300px;height:20px;margin:0 10px 0 5px",
            (5.0, 0.0, 185.0, 20.0),
        ),
        // A maximum above the width leaves it alone, and a minimum wins
        // over a maximum.
        case(
            "a_max_above_width",
            AutoHeight,
            Plain,
            "max-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 50.0, 20.0),
        ),
        case(
            "a_min_wins",
            AutoHeight,
            Plain,
            "min-width:stretch;max-width:100px;width:10px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        // The limit is a size of the box-sizing box: the border and padding
        // of a content-box control come out of the stretched width.
        case(
            "a_min_content_box",
            AutoHeight,
            ContentBox,
            "padding:0 5px;min-width:stretch;width:20px;height:20px",
            (0.0, 0.0, 200.0, 22.0),
        ),
        case(
            "a_max_content_box",
            AutoHeight,
            ContentBox,
            "padding:0 5px;max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 22.0),
        ),
    ]);
}

/// A control in a 200px by 100px block, with the height limited by `stretch`.
#[test]
fn b_the_block_axis_fills_the_definite_height_of_the_parent() {
    check(&[
        case(
            "b_min",
            FixedHeight,
            Plain,
            "min-height:stretch;height:20px;width:50px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "b_max",
            FixedHeight,
            Plain,
            "max-height:stretch;height:300px;width:50px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "b_min_block_size",
            FixedHeight,
            Plain,
            "min-block-size:stretch;height:20px;width:50px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "b_max_block_size",
            FixedHeight,
            Plain,
            "max-block-size:stretch;height:300px;width:50px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "b_both_axes",
            FixedHeight,
            Plain,
            "min-width:stretch;min-height:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 100.0),
        ),
        // The vertical margins come off the room.
        case(
            "b_min_margins",
            FixedHeight,
            Plain,
            "min-height:stretch;height:20px;width:50px;margin:5px 0 3px",
            (0.0, 5.0, 50.0, 92.0),
        ),
        case(
            "b_max_margins",
            FixedHeight,
            Plain,
            "max-height:stretch;height:300px;width:50px;margin:5px 0 3px",
            (0.0, 5.0, 50.0, 92.0),
        ),
        case(
            "b_min_content_box",
            FixedHeight,
            ContentBox,
            "padding:3px 0;min-height:stretch;height:20px;width:50px",
            (0.0, 0.0, 52.0, 100.0),
        ),
        case(
            "b_max_content_box",
            FixedHeight,
            ContentBox,
            "padding:3px 0;max-height:stretch;height:300px;width:50px",
            (0.0, 0.0, 52.0, 100.0),
        ),
    ]);
}

/// A parent whose height follows its content has no room to fill: a minimum is
/// `0` and a maximum is `none`.
#[test]
fn c_an_indefinite_height_leaves_the_block_axis_alone() {
    check(&[
        case(
            "c_min",
            AutoHeight,
            Plain,
            "min-height:stretch;height:20px;width:50px",
            (0.0, 0.0, 50.0, 20.0),
        ),
        case(
            "c_max",
            AutoHeight,
            Plain,
            "max-height:stretch;height:300px;width:50px",
            (0.0, 0.0, 50.0, 300.0),
        ),
    ]);
}

/// The containing block decides the room, whatever it is.
#[test]
fn d_the_room_is_the_one_the_containing_block_leaves() {
    check(&[
        case(
            "d_percentage_width",
            Percent,
            Plain,
            "max-width:stretch;width:900px;height:20px",
            (0.0, 0.0, 400.0, 20.0),
        ),
        case(
            "d_padding",
            Padded,
            Plain,
            "max-width:stretch;width:300px;height:20px",
            (10.0, 0.0, 200.0, 20.0),
        ),
        case(
            "d_right_to_left",
            RightToLeft,
            Plain,
            "max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
    ]);
}

/// A control whose `display` is not the default is clamped the same way.
#[test]
fn e_a_display_of_the_control_changes_nothing() {
    check(&[
        case(
            "e_inline_min",
            AutoHeight,
            Plain,
            "display:inline;min-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_inline_max",
            AutoHeight,
            Plain,
            "display:inline;max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_inline_min_height",
            FixedHeight,
            Plain,
            "display:inline;min-height:stretch;width:50px;height:20px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "e_inline_max_height",
            FixedHeight,
            Plain,
            "display:inline;max-height:stretch;width:50px;height:300px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "e_inline_block_min",
            AutoHeight,
            Plain,
            "display:inline-block;min-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_inline_block_max",
            AutoHeight,
            Plain,
            "display:inline-block;max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_inline_block_min_height",
            FixedHeight,
            Plain,
            "display:inline-block;min-height:stretch;width:50px;height:20px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "e_inline_block_max_height",
            FixedHeight,
            Plain,
            "display:inline-block;max-height:stretch;width:50px;height:300px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        // Block-level, flex items, grid items and a control on a line were
        // clamped before.
        case(
            "e_block_min",
            AutoHeight,
            Plain,
            "display:block;min-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_block_max",
            AutoHeight,
            Plain,
            "display:block;max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_block_min_height",
            FixedHeight,
            Plain,
            "display:block;min-height:stretch;width:50px;height:20px",
            (0.0, 0.0, 50.0, 100.0),
        ),
        case(
            "e_flex_min",
            Flex,
            Plain,
            "min-width:stretch;width:50px;height:20px",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_flex_max",
            Flex,
            Plain,
            "max-width:stretch;width:300px;height:20px;flex:none",
            (0.0, 0.0, 200.0, 20.0),
        ),
        case(
            "e_grid_max",
            Grid,
            Plain,
            "max-width:stretch;width:300px;height:20px",
            (0.0, 0.0, 300.0, 20.0),
        ),
        case(
            "e_on_a_line",
            Line,
            Plain,
            "max-width:stretch;width:300px;height:20px",
            (0.0, 10.0, 200.0, 20.0),
        ),
    ]);
}

/// A MathML root is an atomic inline of the same kind. Chromium sets it on the
/// baseline of its line, which raikiri does not yet, so only the sizes and the
/// left edge are compared.
#[test]
fn f_a_mathml_root_is_clamped_like_a_control() {
    let math = |parent: Parent, style: &str| {
        let element = format!(
            "<math id=a style=\"border:0;padding:0;box-sizing:border-box;{style}\"><mi>x</mi></math>"
        );
        rect_of_a(&wrap(parent, &element))
    };
    let cases: [(Parent, &str, (f32, f32, f32)); 5] = [
        (
            AutoHeight,
            "min-width:stretch;width:50px;height:20px",
            (0.0, 200.0, 20.0),
        ),
        (
            AutoHeight,
            "max-width:stretch;width:300px;height:20px",
            (0.0, 200.0, 20.0),
        ),
        (
            AutoHeight,
            "max-width:stretch;width:300px;height:20px;margin:0 10px 0 5px",
            (5.0, 185.0, 20.0),
        ),
        (
            FixedHeight,
            "min-height:stretch;height:20px;width:50px",
            (0.0, 50.0, 100.0),
        ),
        (
            FixedHeight,
            "max-height:stretch;height:300px;width:50px",
            (0.0, 50.0, 100.0),
        ),
    ];
    for (parent, style, (x, width, height)) in cases {
        let found = math(parent, style);
        assert!(
            close(found.0, x) && close(found.2, width) && close(found.3, height),
            "<math style=\"{style}\"> is {found:?}, expected x {x}, size {width}x{height}"
        );
    }
}

/// A minimum or maximum that is an intrinsic sizing keyword is the size
/// `width` takes from that keyword. The intrinsic sizes of the controls
/// themselves are a separate matter (an `input` has none), so the rows hold
/// the limit against the `width` keyword of the same control.
#[test]
fn g_an_intrinsic_keyword_limit_is_the_size_the_width_keyword_gives() {
    let content = |tag: &str| match tag {
        "textarea" | "button" => "abc",
        "select" => "<option>abcdef</option>",
        _ => "",
    };
    let width_of = |tag: &str, style: &str| {
        let body = if tag == "input" {
            format!(
                "<input id=a style=\"border:0;padding:0;box-sizing:border-box;height:20px;{style}\">"
            )
        } else if tag == "math" {
            format!(
                "<math id=a style=\"border:0;padding:0;box-sizing:border-box;height:20px;{style}\"><mi>x</mi></math>"
            )
        } else {
            format!(
                "<{tag} id=a style=\"border:0;padding:0;box-sizing:border-box;height:20px;{style}\">{}</{tag}>",
                content(tag)
            )
        };
        rect_of_a(&wrap(AutoHeight, &body)).2
    };
    for tag in ["input", "textarea", "select", "button", "math"] {
        for keyword in ["min-content", "max-content", "fit-content"] {
            let own = width_of(tag, &format!("width:{keyword}"));
            let capped = width_of(tag, &format!("width:900px;max-width:{keyword}"));
            let raised = width_of(tag, &format!("width:5px;min-width:{keyword}"));
            assert!(
                close(capped, own),
                "<{tag}> with max-width: {keyword} is {capped} wide, width: {keyword} gives {own}"
            );
            assert!(
                close(raised, own.max(5.0)),
                "<{tag}> with min-width: {keyword} is {raised} wide, width: {keyword} gives {own}"
            );
        }
    }
}
