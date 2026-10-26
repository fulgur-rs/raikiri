//! The static position of an out-of-flow box among the in-flow content of its
//! containing block in every writing mode (CSS 2.1 10.6.4, CSS Position 3
//! 4.3): below the blocks and the lines that come before it, aligned in the
//! zero-sized rectangle it leaves in the block axis. The positions were
//! measured in Chromium.

use crate::Document;
use crate::layout::ifc::test_support::sheet_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

const BOX: &str = "position:absolute;width:30px;height:20px;";

/// What the root holds, in order.
#[derive(Clone, Copy)]
enum Child {
    /// A run of text.
    Text(&'static str),
    /// A block holding text, with the given style.
    Block(&'static str, &'static str),
    /// The out-of-flow `div` under test, with the given style.
    Box(&'static str),
    /// The out-of-flow `span` under test, with the given style.
    Inline(&'static str),
    /// A forced line break.
    Break,
    /// An inline block of 10x10.
    Atomic,
    /// A positioned `span` holding the children.
    Relative(&'static [Child]),
    /// A `span` with the given style, holding the children.
    Styled(&'static str, &'static [Child]),
}

/// The root of the paragraph under test.
#[derive(Clone, Copy)]
struct Setup {
    /// Author style sheet; the root has the id `cb`.
    sheet: &'static str,
    /// Style of the root besides its size, position and writing mode.
    root_css: &'static str,
}

fn append(doc: &mut Document, parent: usize, child: Child, tested: &mut usize) {
    match child {
        Child::Text(text) => {
            doc.append_text(parent, text);
        }
        Child::Block(text, css) => {
            let block =
                doc.append_element(Some(parent), "div", Some(&format!("display:block;{css}")));
            doc.append_text(block, text);
        }
        Child::Box(css) => {
            *tested = doc.append_element(
                Some(parent),
                "div",
                Some(&format!("display:block;{BOX}{css}")),
            );
        }
        Child::Inline(css) => {
            *tested = doc.append_element(Some(parent), "span", Some(&format!("{BOX}{css}")));
        }
        Child::Break => {
            doc.append_element(Some(parent), "br", None::<&str>);
        }
        Child::Atomic => {
            doc.append_element(
                Some(parent),
                "span",
                Some("display:inline-block;width:10px;height:10px"),
            );
        }
        Child::Relative(children) => {
            let span = doc.append_element(Some(parent), "span", Some("position:relative"));
            for &child in children {
                append(doc, span, child, tested);
            }
        }
        Child::Styled(css, children) => {
            let span = doc.append_element(Some(parent), "span", Some(css));
            for &child in children {
                append(doc, span, child, tested);
            }
        }
    }
}

/// A border box as `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The writing mode, the direction and the box expected under them.
type Expected<'a> = (&'a str, &'a str, Rect);

/// The border box of the out-of-flow box among `children`, relative to the
/// border box of the 200x100 root in the writing mode `mode` and the
/// direction `direction`.
fn placed(mode: &str, direction: &str, setup: Setup, children: &[Child]) -> Rect {
    let root_css = format!(
        "position:relative;writing-mode:{mode};direction:{direction};width:200px;\
         height:100px;box-sizing:border-box;line-height:10px;{}",
        setup.root_css
    );
    let mut tested = 0;
    let fixture = sheet_fixture(setup.sheet, &root_css, |doc, root| {
        doc.set_element_attribute(root, "id", "cb")
            .expect("set the root id");
        for &child in children {
            append(doc, root, child, &mut tested);
        }
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (root_x, root_y, ..) = absolute_rect(&doc, root);
    let (x, y, width, height) = absolute_rect(&doc, tested);
    (x - root_x, y - root_y, width, height)
}

/// Check the box among `children` against the position expected in each
/// writing mode and direction.
fn check(name: &str, setup: Setup, children: &[Child], expected: &[Expected]) {
    for &(mode, direction, rect) in expected {
        assert_eq!(
            placed(mode, direction, setup, children),
            rect,
            "{name} in {mode} {direction}"
        );
    }
}

#[test]
fn an_out_of_flow_box_after_block_children_follows_the_blocks() {
    check(
        "block_then_box",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Block("AAAA", ""), Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "box_then_block",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Box(""), Child::Block("AAAA", "")],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "block_text_box_block",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", ""),
            Child::Text("CC"),
            Child::Box(""),
            Child::Block("DDDD", ""),
        ],
        &[
            ("horizontal-tb", "ltr", (0.0, 20.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 20.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (150.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (150.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (20.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (20.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (20.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (20.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "block_box_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Block("AAAA", ""), Child::Box(""), Child::Text("CC")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "margins_before",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", "margin:3px 5px 7px 9px"),
            Child::Box(""),
        ],
        &[
            ("horizontal-tb", "ltr", (0.0, 17.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 17.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (146.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (146.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (24.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (24.0, 80.0, 30.0, 20.0)),
        ],
    );
    check(
        "own_margins",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", ""),
            Child::Box("margin:2px 4px 6px 8px"),
        ],
        &[
            ("horizontal-tb", "ltr", (8.0, 12.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (166.0, 12.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (156.0, 2.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (156.0, 74.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (18.0, 2.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (18.0, 74.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (18.0, 74.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (18.0, 2.0, 30.0, 20.0)),
        ],
    );
    check(
        "padded_root",
        Setup {
            sheet: r#""#,
            root_css: "padding:5px 7px 11px 13px;border:2px solid;",
        },
        &[Child::Block("AAAA", ""), Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (15.0, 17.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (161.0, 17.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (151.0, 7.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (151.0, 67.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (25.0, 7.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (25.0, 67.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (25.0, 67.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (25.0, 7.0, 30.0, 20.0)),
        ],
    );
    check(
        "inset_one_axis",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Block("AAAA", ""), Child::Box("right:3px")],
        &[
            ("horizontal-tb", "ltr", (167.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (167.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (167.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (167.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (167.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (167.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (167.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (167.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "inset_other_axis",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Block("AAAA", ""), Child::Box("top:3px")],
        &[
            ("horizontal-tb", "ltr", (0.0, 3.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 3.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 3.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 3.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 3.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 3.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 3.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 3.0, 30.0, 20.0)),
        ],
    );
    check(
        "multicol_block_then_box",
        Setup {
            sheet: r#""#,
            root_css: "column-count:2;column-gap:0;",
        },
        &[Child::Block("AAAA", ""), Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_out_of_flow_inline_alone_after_a_break_follows_the_lines_above_it() {
    // The line the box would have started holds nothing else, so it has no
    // anchor: the box sits below the last line before it, not below the block.
    const AFTER_THE_TEXT: &[Expected] = &[
        ("horizontal-tb", "ltr", (0.0, 20.0, 30.0, 20.0)),
        ("horizontal-tb", "rtl", (170.0, 20.0, 30.0, 20.0)),
        ("vertical-rl", "ltr", (150.0, 0.0, 30.0, 20.0)),
        ("vertical-rl", "rtl", (150.0, 80.0, 30.0, 20.0)),
        ("vertical-lr", "ltr", (20.0, 0.0, 30.0, 20.0)),
        ("vertical-lr", "rtl", (20.0, 80.0, 30.0, 20.0)),
        ("sideways-lr", "ltr", (20.0, 80.0, 30.0, 20.0)),
        ("sideways-lr", "rtl", (20.0, 0.0, 30.0, 20.0)),
    ];
    let setup = Setup {
        sheet: r#""#,
        root_css: "",
    };
    check(
        "block_text_break_inline",
        setup,
        &[
            Child::Block("AAAA", ""),
            Child::Text("CC"),
            Child::Break,
            Child::Inline(""),
        ],
        AFTER_THE_TEXT,
    );
    check(
        "block_text_break_inline_block",
        setup,
        &[
            Child::Block("AAAA", ""),
            Child::Text("CC"),
            Child::Break,
            Child::Inline(""),
            Child::Block("DDDD", ""),
        ],
        AFTER_THE_TEXT,
    );
    check(
        "text_break_inline",
        setup,
        &[Child::Text("CC"), Child::Break, Child::Inline("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_out_of_flow_box_below_text_only_when_something_precedes_it_on_the_line() {
    check(
        "text_then_box",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Text("AAAA"), Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "box_then_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Box(""), Child::Text("AAAA")],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "box_between_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Text("AA"), Child::Box(""), Child::Text("AA")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "text_indent",
        Setup {
            sheet: r#""#,
            root_css: "text-indent:20px;",
        },
        &[Child::Text("AA"), Child::Box(""), Child::Text("AA")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "atomic_then_box",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Atomic, Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 12.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 12.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (12.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (12.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "atomic_after_box",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Box(""), Child::Atomic],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "generated_block_then_box",
        Setup {
            sheet: r#"#cb::before{content:"BB";display:block}"#,
            root_css: "",
        },
        &[Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "generated_block_text_box",
        Setup {
            sheet: r#"#cb::before{content:"BB";display:block}"#,
            root_css: "",
        },
        &[Child::Text("AA"), Child::Box("")],
        &[
            ("horizontal-tb", "ltr", (0.0, 20.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 20.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (150.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (150.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (20.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (20.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (20.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (20.0, 0.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_out_of_flow_box_is_aligned_in_its_static_position_rectangle() {
    check(
        "center",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", ""),
            Child::Box("align-self:center;justify-self:center"),
        ],
        &[
            ("vertical-rl", "ltr", (175.0, 40.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (175.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (-5.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (-5.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (-5.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (-5.0, 40.0, 30.0, 20.0)),
        ],
    );
    check(
        "end",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", ""),
            Child::Box("align-self:end;justify-self:end"),
        ],
        &[
            ("vertical-rl", "ltr", (190.0, 80.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (190.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (-20.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (-20.0, 0.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (-20.0, 0.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (-20.0, 80.0, 30.0, 20.0)),
        ],
    );
    check(
        "self_start_end",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Block("AAAA", ""),
            Child::Box("align-self:start;justify-self:end"),
        ],
        &[
            ("horizontal-tb", "ltr", (170.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (0.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_out_of_flow_inline_stays_at_its_anchor_and_reads_with_the_text() {
    check(
        "inline_in_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Text("AAAA"), Child::Inline(""), Child::Text("AA")],
        &[
            ("horizontal-tb", "ltr", (40.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (180.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 40.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "relative_inline_containing_block",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Text("AA"),
            Child::Relative(&[Child::Text("XX"), Child::Box(""), Child::Text("YY")]),
        ],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_out_of_flow_inline_extends_the_way_the_text_at_its_anchor_reads() {
    check(
        "digits_end",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Text("1 2 34"), Child::Inline("")],
        &[
            ("horizontal-tb", "ltr", (60.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (110.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 60.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 20.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 60.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 20.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 20.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 60.0, 30.0, 20.0)),
        ],
    );
    check(
        "digits_start",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Inline(""), Child::Text("1 2 34")],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "letters_digits",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Text("AA"), Child::Inline(""), Child::Text("34")],
        &[
            ("horizontal-tb", "ltr", (20.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (180.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 20.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 20.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 60.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "rtl_override_between",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:rtl",
            &[Child::Text("AAAA"), Child::Inline(""), Child::Text("AA")],
        )],
        &[
            ("horizontal-tb", "ltr", (-10.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (130.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 40.0, 30.0, 20.0)),
        ],
    );
    check(
        "ltr_override_between",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:ltr",
            &[Child::Text("AAAA"), Child::Inline(""), Child::Text("AA")],
        )],
        &[
            ("horizontal-tb", "ltr", (40.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (180.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 40.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "rtl_override_then_letters",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[
            Child::Styled(
                "unicode-bidi:bidi-override;direction:rtl",
                &[Child::Text("AAAA")],
            ),
            Child::Inline(""),
            Child::Text("AA"),
        ],
        &[
            ("horizontal-tb", "ltr", (40.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (130.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 40.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 40.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 40.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn a_block_level_box_follows_the_text_before_it_in_visual_order() {
    check(
        "block_after_rtl_override_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:rtl",
            &[Child::Text("AAAA"), Child::Box("")],
        )],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "block_after_rtl_override_atomic",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:rtl",
            &[Child::Atomic, Child::Box("")],
        )],
        &[
            ("horizontal-tb", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 12.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (0.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (12.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "block_before_rtl_override_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:rtl",
            &[Child::Box(""), Child::Text("AAAA")],
        )],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (170.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (0.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (0.0, 0.0, 30.0, 20.0)),
        ],
    );
    check(
        "block_between_rtl_override_text",
        Setup {
            sheet: r#""#,
            root_css: "",
        },
        &[Child::Styled(
            "unicode-bidi:bidi-override;direction:rtl",
            &[Child::Text("AA"), Child::Box(""), Child::Text("AA")],
        )],
        &[
            ("horizontal-tb", "ltr", (0.0, 10.0, 30.0, 20.0)),
            ("horizontal-tb", "rtl", (170.0, 10.0, 30.0, 20.0)),
            ("vertical-rl", "ltr", (160.0, 0.0, 30.0, 20.0)),
            ("vertical-rl", "rtl", (160.0, 80.0, 30.0, 20.0)),
            ("vertical-lr", "ltr", (10.0, 0.0, 30.0, 20.0)),
            ("vertical-lr", "rtl", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "ltr", (10.0, 80.0, 30.0, 20.0)),
            ("sideways-lr", "rtl", (10.0, 0.0, 30.0, 20.0)),
        ],
    );
}
