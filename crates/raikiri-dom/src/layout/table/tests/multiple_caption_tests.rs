//! A table with several captions.
//!
//! Captions above the grid stack in source order from the block start, and
//! captions below it stack in source order after the grid, whatever their
//! place among the table's children. Every rectangle is what Chromium reports
//! for the same markup (Ahem 10px, no body margin, `border-spacing: 0`).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// A rectangle as (left, top, width, height).
type Rect = (f32, f32, f32, f32);

/// A child of the table.
#[derive(Clone, Copy)]
enum Part {
    /// A caption with this extra CSS, over `display:table-caption`.
    Caption(&'static str),
    /// A caption inside a `display: contents` wrapper.
    WrappedCaption(&'static str),
    /// A 10px high caption with this text.
    TextCaption(&'static str),
    /// A row with one 30x10 cell.
    Row,
}

struct Laid {
    doc: Document,
    table: usize,
    captions: Vec<usize>,
    cell: usize,
}

impl Laid {
    fn table(&self) -> Rect {
        absolute_rect(&self.doc, self.table)
    }

    fn captions(&self) -> Vec<Rect> {
        self.captions
            .iter()
            .map(|&caption| absolute_rect(&self.doc, caption))
            .collect()
    }

    fn cell(&self) -> Rect {
        absolute_rect(&self.doc, self.cell)
    }
}

/// `body(margin:0) > table(table_css) > parts` laid out on an 800x600 page.
fn lay_out(table_css: &str, parts: &[Part]) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let table = doc.append_element(
        Some(body),
        "div",
        Some(&format!("display:table;border-spacing:0;{table_css}")),
    );
    let mut captions = Vec::new();
    let mut cell = None;
    for &part in parts {
        match part {
            Part::Caption(css) | Part::WrappedCaption(css) | Part::TextCaption(css) => {
                let parent = if matches!(part, Part::WrappedCaption(_)) {
                    doc.append_element(Some(table), "div", Some("display:contents"))
                } else {
                    table
                };
                let (css, text) = if matches!(part, Part::TextCaption(_)) {
                    ("height:10px", css)
                } else {
                    (css, "a")
                };
                let caption = doc.append_element(
                    Some(parent),
                    "div",
                    Some(&format!("display:table-caption;{css}")),
                );
                doc.append_text(caption, text);
                captions.push(caption);
            }
            Part::Row => {
                let row = doc.append_element(Some(table), "div", Some("display:table-row"));
                cell = Some(doc.append_element(
                    Some(row),
                    "div",
                    Some("display:table-cell;padding:0;width:30px;height:10px"),
                ));
            }
        }
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid {
        doc,
        table,
        captions,
        cell: cell.expect("the table has a row"),
    }
}

#[test]
fn two_captions_above_the_grid_stack_in_source_order() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("height:10px"),
            Part::Caption("height:20px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 40.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 30.0, 10.0), (0.0, 10.0, 30.0, 20.0)]
    );
    assert_eq!(laid.cell(), (0.0, 30.0, 30.0, 10.0));
}

#[test]
fn two_captions_below_the_grid_stack_in_source_order() {
    let laid = lay_out(
        "",
        &[
            Part::Row,
            Part::Caption("caption-side:bottom;height:10px"),
            Part::Caption("caption-side:bottom;height:20px"),
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 40.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 10.0, 30.0, 10.0), (0.0, 20.0, 30.0, 20.0)]
    );
    assert_eq!(laid.cell(), (0.0, 0.0, 30.0, 10.0));
}

#[test]
fn a_bottom_caption_ahead_of_two_top_captions_stays_below_the_grid() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("caption-side:bottom;height:10px"),
            Part::Caption("height:20px"),
            Part::Caption("height:30px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 70.0));
    assert_eq!(
        laid.captions(),
        [
            (0.0, 60.0, 30.0, 10.0),
            (0.0, 0.0, 30.0, 20.0),
            (0.0, 20.0, 30.0, 30.0)
        ]
    );
    assert_eq!(laid.cell(), (0.0, 50.0, 30.0, 10.0));
}

#[test]
fn the_place_of_the_rows_among_the_captions_does_not_matter() {
    let laid = lay_out(
        "",
        &[
            Part::Row,
            Part::Caption("height:10px"),
            Part::Caption("caption-side:bottom;height:20px"),
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 40.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 30.0, 10.0), (0.0, 20.0, 30.0, 20.0)]
    );
    assert_eq!(laid.cell(), (0.0, 10.0, 30.0, 10.0));
}

#[test]
fn a_wide_second_caption_widens_the_table_and_every_caption() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("height:10px"),
            Part::Caption("height:10px;width:100px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 100.0, 30.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 100.0, 10.0), (0.0, 10.0, 100.0, 10.0)]
    );
    assert_eq!(laid.cell(), (0.0, 20.0, 100.0, 10.0));
}

#[test]
fn a_table_is_as_wide_as_the_widest_min_content_among_its_captions() {
    // Ahem 10px: "aaaaa" is 50px and "bbbbbbbb" is 80px, whichever comes first.
    for order in [["aaaaa", "bbbbbbbb"], ["bbbbbbbb", "aaaaa"]] {
        let [first, second] = order;
        let laid = lay_out(
            "",
            &[
                Part::TextCaption(first),
                Part::TextCaption(second),
                Part::Row,
            ],
        );
        assert_eq!(laid.table(), (0.0, 0.0, 80.0, 30.0), "{order:?}");
        assert_eq!(
            laid.captions(),
            [(0.0, 0.0, 80.0, 10.0), (0.0, 10.0, 80.0, 10.0)],
            "{order:?}"
        );
        assert_eq!(laid.cell(), (0.0, 20.0, 80.0, 10.0), "{order:?}");
    }
}

#[test]
fn a_caption_that_states_its_own_width_keeps_it_beside_a_wider_one() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("height:10px;width:min-content"),
            Part::Caption("height:10px;width:80px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 80.0, 30.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 10.0, 10.0), (0.0, 10.0, 80.0, 10.0)]
    );
    assert_eq!(laid.cell(), (0.0, 20.0, 80.0, 10.0));
}

#[test]
fn a_stated_table_width_is_the_width_of_every_caption() {
    let laid = lay_out(
        "width:100px",
        &[
            Part::Caption("height:10px"),
            Part::Caption("height:10px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 100.0, 30.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 100.0, 10.0), (0.0, 10.0, 100.0, 10.0)]
    );
    assert_eq!(laid.cell(), (0.0, 20.0, 100.0, 10.0));
}

#[test]
fn caption_margins_add_up_without_collapsing() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("height:10px;margin:5px 0 7px"),
            Part::Caption("height:10px;margin:3px 2px 4px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 49.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 5.0, 30.0, 10.0), (2.0, 25.0, 26.0, 10.0)]
    );
    assert_eq!(laid.cell(), (0.0, 39.0, 30.0, 10.0));
}

#[test]
fn a_caption_inside_a_contents_wrapper_stacks_with_the_others() {
    let laid = lay_out(
        "",
        &[
            Part::Caption("height:10px"),
            Part::WrappedCaption("height:20px"),
            Part::Row,
        ],
    );
    assert_eq!(laid.table(), (0.0, 0.0, 30.0, 40.0));
    assert_eq!(
        laid.captions(),
        [(0.0, 0.0, 30.0, 10.0), (0.0, 10.0, 30.0, 20.0)]
    );
    assert_eq!(laid.cell(), (0.0, 30.0, 30.0, 10.0));
}

#[test]
fn an_absolutely_positioned_caption_takes_no_room_whichever_one_it_is() {
    let second = lay_out(
        "position:relative",
        &[
            Part::Caption("height:10px"),
            Part::Caption("position:absolute;top:0;left:0;width:20px;height:20px"),
            Part::Row,
        ],
    );
    assert_eq!(second.table(), (0.0, 0.0, 30.0, 20.0));
    assert_eq!(
        second.captions(),
        [(0.0, 0.0, 30.0, 10.0), (0.0, 0.0, 20.0, 20.0)]
    );
    assert_eq!(second.cell(), (0.0, 10.0, 30.0, 10.0));

    let first = lay_out(
        "position:relative",
        &[
            Part::Caption("position:absolute;top:0;left:0;width:20px;height:20px"),
            Part::Caption("height:10px"),
            Part::Row,
        ],
    );
    assert_eq!(first.table(), (0.0, 0.0, 30.0, 20.0));
    assert_eq!(
        first.captions(),
        [(0.0, 0.0, 20.0, 20.0), (0.0, 0.0, 30.0, 10.0)]
    );
    assert_eq!(first.cell(), (0.0, 10.0, 30.0, 10.0));
}

#[test]
fn vertical_lr_captions_stack_from_the_left_edge() {
    let tops = lay_out(
        "writing-mode:vertical-lr",
        &[
            Part::Caption("width:10px"),
            Part::Caption("width:20px"),
            Part::Row,
        ],
    );
    assert_eq!(tops.table(), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(
        tops.captions(),
        [(0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 20.0, 10.0)]
    );
    assert_eq!(tops.cell(), (30.0, 0.0, 30.0, 10.0));

    let mixed = lay_out(
        "writing-mode:vertical-lr",
        &[
            Part::Caption("width:10px"),
            Part::Caption("caption-side:bottom;width:20px"),
            Part::Row,
        ],
    );
    assert_eq!(mixed.table(), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(
        mixed.captions(),
        [(0.0, 0.0, 10.0, 10.0), (40.0, 0.0, 20.0, 10.0)]
    );
    assert_eq!(mixed.cell(), (10.0, 0.0, 30.0, 10.0));
}

#[test]
fn vertical_rl_captions_stack_from_the_right_edge() {
    let tops = lay_out(
        "writing-mode:vertical-rl",
        &[
            Part::Caption("width:10px"),
            Part::Caption("width:20px"),
            Part::Row,
        ],
    );
    assert_eq!(tops.table(), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(
        tops.captions(),
        [(50.0, 0.0, 10.0, 10.0), (30.0, 0.0, 20.0, 10.0)]
    );
    assert_eq!(tops.cell(), (0.0, 0.0, 30.0, 10.0));

    let mixed = lay_out(
        "writing-mode:vertical-rl",
        &[
            Part::Caption("width:10px"),
            Part::Caption("caption-side:bottom;width:20px"),
            Part::Caption("caption-side:bottom;width:30px"),
            Part::Row,
        ],
    );
    assert_eq!(mixed.table(), (0.0, 0.0, 90.0, 10.0));
    assert_eq!(
        mixed.captions(),
        [
            (80.0, 0.0, 10.0, 10.0),
            (30.0, 0.0, 20.0, 10.0),
            (0.0, 0.0, 30.0, 10.0)
        ]
    );
    assert_eq!(mixed.cell(), (50.0, 0.0, 30.0, 10.0));
}

#[test]
fn sideways_modes_stack_captions_like_the_vertical_ones() {
    let parts = [
        Part::Caption("width:10px"),
        Part::Caption("caption-side:bottom;width:20px"),
        Part::Caption("caption-side:bottom;width:30px"),
        Part::Row,
    ];
    let rl = lay_out("writing-mode:sideways-rl", &parts);
    assert_eq!(rl.table(), (0.0, 0.0, 90.0, 10.0));
    assert_eq!(
        rl.captions(),
        [
            (80.0, 0.0, 10.0, 10.0),
            (30.0, 0.0, 20.0, 10.0),
            (0.0, 0.0, 30.0, 10.0)
        ]
    );
    assert_eq!(rl.cell(), (50.0, 0.0, 30.0, 10.0));

    let lr = lay_out("writing-mode:sideways-lr", &parts);
    assert_eq!(lr.table(), (0.0, 0.0, 90.0, 10.0));
    assert_eq!(
        lr.captions(),
        [
            (0.0, 0.0, 10.0, 10.0),
            (40.0, 0.0, 20.0, 10.0),
            (60.0, 0.0, 30.0, 10.0)
        ]
    );
    assert_eq!(lr.cell(), (10.0, 0.0, 30.0, 10.0));
}
