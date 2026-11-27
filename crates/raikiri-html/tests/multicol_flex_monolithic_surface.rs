//! Boxes with size containment in the block axis inside a flex container in
//! a multi-column container: a column end never cuts such a monolithic box,
//! so one that does not fit moves to the next column, and the boxes around
//! it grow by the space it leaves (CSS Fragmentation 3, 4.4; CSS Flexbox 1,
//! 12). The values are the ones Chromium gives for the same documents.

use crate::multicol_flex_forced_break_surface::{columns, expect};

#[test]
fn a_monolithic_box_that_does_not_fit_moves_to_the_next_column_and_its_item_grows() {
    // A box with size containment that starts below the top of a column and does
    // not fit moves to the next one; the item holding it grows by the space
    // it leaves, and the item after it moves down with it.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-direction:column'><div id=a>\
             <div id=m1 style='contain:size;height:60px'></div>\
             <div id=m2 style='contain:size;height:60px'></div></div>\
             <div id=b style='height:30px'></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 90)]),
            ("a", &[(0, 0, 100, 100), (100, 0, 100, 60)]),
            ("m1", &[(0, 0, 100, 60)]),
            ("m2", &[(100, 0, 100, 60)]),
            ("b", &[(100, 60, 100, 30)]),
        ],
    );
}

#[test]
fn a_monolithic_first_box_moves_its_item() {
    // A break before the first box of an item is one before the item, so the
    // whole item moves, with its padding.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-direction:column'>\
             <div id=a style='height:70px'></div><div id=b style='padding-top:5px'>\
             <div id=m1 style='contain:size;height:40px'></div>\
             <div id=b1 style='height:10px'></div></div>\
             <div id=c style='height:10px'></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 65)]),
            ("a", &[(0, 0, 100, 70)]),
            ("b", &[(100, 0, 100, 55)]),
            ("m1", &[(100, 5, 100, 40)]),
            ("b1", &[(100, 45, 100, 10)]),
            ("c", &[(100, 55, 100, 10)]),
        ],
    );
}

#[test]
fn a_row_line_grows_and_stretches_its_items() {
    // In a row container the line grows with the item: items that stretch
    // grow with it, an item aligned at the start keeps its size.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex'><div id=a style='width:50px'>\
             <div id=m1 style='contain:size;height:60px'></div>\
             <div id=m2 style='contain:size;height:60px'></div></div>\
             <div id=b style='width:50px'></div>\
             <div id=c style='width:50px;height:30px;align-self:flex-start'></div></div>\
             <div id=o style='height:10px'></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 60)]),
            ("a", &[(0, 0, 33, 100), (100, 0, 33, 60)]),
            ("m1", &[(0, 0, 33, 60)]),
            ("m2", &[(100, 0, 33, 60)]),
            ("b", &[(33, 0, 33, 100), (133, 0, 33, 60)]),
            ("c", &[(67, 0, 33, 30)]),
            ("o", &[(100, 60, 100, 10)]),
        ],
    );
}

#[test]
fn a_monolithic_box_starting_a_flex_line_moves_the_line() {
    // A monolithic box at the start of an item in a later flex line moves the
    // whole line, and the lines after it.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-wrap:wrap'>\
             <div id=a style='width:100px;height:70px'></div><div id=b style='width:50px'>\
             <div id=m1 style='contain:size;height:40px'></div></div>\
             <div id=c style='width:50px;height:10px'></div>\
             <div id=d style='width:100px;height:10px'></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 50)]),
            ("a", &[(0, 0, 100, 70)]),
            ("b", &[(100, 0, 50, 40)]),
            ("m1", &[(100, 0, 50, 40)]),
            ("c", &[(150, 0, 50, 10)]),
            ("d", &[(100, 40, 100, 10)]),
        ],
    );
}

#[test]
fn an_item_with_a_block_size_of_its_own_does_not_grow() {
    // An item with a height keeps it: the moved box overflows it.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex'><div id=a style='width:25px;height:110px'>\
             <div id=m1 style='contain:size;height:80px'></div>\
             <div id=m2 style='contain:size;height:30px'></div></div>\
             <div id=b style='width:25px;height:200px'></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 100)]),
            ("a", &[(0, 0, 25, 100), (100, 0, 25, 10)]),
            ("m1", &[(0, 0, 25, 80)]),
            ("m2", &[(100, 0, 25, 30)]),
            ("b", &[(25, 0, 25, 100), (125, 0, 25, 100)]),
        ],
    );
}

#[test]
fn balanced_columns_are_as_tall_as_the_tallest_monolithic_box() {
    // Balanced columns are at least as tall as a box they cannot break, and
    // as tall as the moved boxes need.
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex'><div id=a style='width:25px'></div>\
             <div id=b style='width:25px'><div id=m1 style='contain:size;height:70px'></div>\
             <div id=m2 style='contain:size;height:100px'></div></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 100)]),
            ("a", &[(0, 0, 25, 100), (100, 0, 25, 100)]),
            ("b", &[(25, 0, 25, 100), (125, 0, 25, 100)]),
            ("m1", &[(25, 0, 25, 70)]),
            ("m2", &[(125, 0, 25, 100)]),
        ],
    );
}

#[test]
fn items_move_in_flex_order() {
    // The items of a column container are laid out in `order`.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-direction:column'><div id=a>\
             <div id=m1 style='contain:size;height:60px'></div>\
             <div id=m2 style='contain:size;height:30px'></div></div>\
             <div id=b style='order:-1'><div id=m3 style='contain:size;height:80px'></div>\
             <div id=m4 style='contain:size;height:30px'></div></div></div>",
        ),
        &[
            (
                "f",
                &[(0, 0, 100, 100), (100, 0, 100, 100), (200, 0, 100, 30)],
            ),
            ("a", &[(100, 30, 100, 70), (200, 0, 100, 30)]),
            ("m1", &[(100, 30, 100, 60)]),
            ("m2", &[(200, 0, 100, 30)]),
            ("b", &[(0, 0, 100, 100), (100, 0, 100, 30)]),
            ("m3", &[(0, 0, 100, 80)]),
            ("m4", &[(100, 0, 100, 30)]),
        ],
    );
}

#[test]
fn a_monolithic_box_inside_nested_blocks_moves() {
    // A monolithic box deep inside blocks moves with the boxes after it, and
    // its margin at the break is truncated.
    expect(
        &columns(
            "auto",
            "<div id=o style='height:20px'></div>\
             <div id=f style='display:flex;flex-direction:column;padding-top:5px'><div id=a>\
             <div id=w><div id=m1 style='contain:size;height:50px'></div>\
             <div id=t style='height:10px'></div>\
             <div id=m2 style='contain:size;height:40px;margin-top:5px'></div></div>\
             <div id=a1 style='height:10px'></div></div>\
             <div id=b style='height:10px'></div></div>",
        ),
        &[
            ("o", &[(0, 0, 100, 20)]),
            ("f", &[(0, 20, 100, 80), (100, 0, 100, 60)]),
            ("a", &[(0, 25, 100, 75), (100, 0, 100, 50)]),
            ("w", &[(0, 25, 100, 75), (100, 0, 100, 40)]),
            ("m1", &[(0, 25, 100, 50)]),
            ("t", &[(0, 75, 100, 10)]),
            ("m2", &[(100, 0, 100, 40)]),
            ("a1", &[(100, 40, 100, 10)]),
            ("b", &[(100, 50, 100, 10)]),
        ],
    );
}

#[test]
fn balanced_columns_stop_at_the_container_height() {
    // Balanced columns stop growing at the height of the container; what
    // does not fit continues in overflow columns.
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex;flex-direction:column'><div id=a>\
             <div id=m1 style='contain:size;height:60px'></div>\
             <div id=m2 style='contain:size;height:60px'></div>\
             <div id=m3 style='contain:size;height:60px'></div>\
             <div id=m4 style='contain:size;height:60px'></div>\
             <div id=m5 style='contain:size;height:60px'></div></div></div>",
        ),
        &[
            (
                "f",
                &[
                    (0, 0, 100, 100),
                    (100, 0, 100, 100),
                    (200, 0, 100, 100),
                    (300, 0, 100, 100),
                    (400, 0, 100, 60),
                ],
            ),
            (
                "a",
                &[
                    (0, 0, 100, 100),
                    (100, 0, 100, 100),
                    (200, 0, 100, 100),
                    (300, 0, 100, 100),
                    (400, 0, 100, 60),
                ],
            ),
            ("m1", &[(0, 0, 100, 60)]),
            ("m2", &[(100, 0, 100, 60)]),
            ("m3", &[(200, 0, 100, 60)]),
            ("m4", &[(300, 0, 100, 60)]),
            ("m5", &[(400, 0, 100, 60)]),
        ],
    );
}
#[test]
fn a_monolithic_box_starting_the_container_moves_the_container() {
    // A monolithic box at the start of the flex container moves the whole
    // container to the next column.
    expect(
        &columns(
            "auto",
            "<div id=o style='height:60px'></div>\
             <div id=f style='display:flex;flex-direction:column'><div id=a>\
             <div id=m1 style='contain:size;height:60px'></div>\
             <div id=b style='height:10px'></div></div></div>",
        ),
        &[
            ("o", &[(0, 0, 100, 60)]),
            ("f", &[(100, 0, 100, 70)]),
            ("a", &[(100, 0, 100, 70)]),
            ("m1", &[(100, 0, 100, 60)]),
            ("b", &[(100, 60, 100, 10)]),
        ],
    );
}

#[test]
fn a_line_moves_before_the_boxes_inside_its_items() {
    // A monolithic box at the start of an item moves its whole line, and the
    // boxes in the other items of the line move with it, not on their own.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-direction:column'>\
             <div id=o style='height:70px'></div><div id=r style='display:flex'>\
             <div id=a style='width:50px'><div id=a1 style='height:5px'></div>\
             <div id=m1 style='contain:size;height:30px'></div></div>\
             <div id=b style='width:50px'>\
             <div id=m2 style='contain:size;height:40px'></div></div></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 40)]),
            ("o", &[(0, 0, 100, 70)]),
            ("r", &[(100, 0, 100, 40)]),
            ("a", &[(100, 0, 50, 40)]),
            ("a1", &[(100, 0, 50, 5)]),
            ("m1", &[(100, 5, 50, 30)]),
            ("b", &[(150, 0, 50, 40)]),
            ("m2", &[(150, 0, 50, 40)]),
        ],
    );
}

#[test]
fn an_item_below_the_start_of_its_line_moves_alone() {
    // An item that its margin keeps below the start of its line moves alone,
    // with its margin truncated; the items beside it stay.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-wrap:wrap'>\
             <div id=a style='width:100px;height:70px'></div>\
             <div id=b style='width:50px;height:10px'></div>\
             <div id=c style='width:50px;margin-top:10px'>\
             <div id=m1 style='contain:size;height:30px'></div></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 30)]),
            ("a", &[(0, 0, 100, 70)]),
            ("b", &[(0, 70, 50, 10)]),
            ("c", &[(150, 0, 50, 30)]),
            ("m1", &[(150, 0, 50, 30)]),
        ],
    );
}

#[test]
fn a_line_starting_the_container_moves_the_container() {
    // A break before the only line of a row container is one before the
    // container.
    expect(
        &columns(
            "auto",
            "<div id=o style='height:70px'></div><div id=f style='display:flex'>\
             <div id=a style='width:50px'><div id=a1 style='height:5px'></div></div>\
             <div id=b style='width:50px'>\
             <div id=m2 style='contain:size;height:40px'></div></div></div>",
        ),
        &[
            ("o", &[(0, 0, 100, 70)]),
            ("f", &[(100, 0, 100, 40)]),
            ("a", &[(100, 0, 50, 40)]),
            ("a1", &[(100, 0, 50, 5)]),
            ("b", &[(150, 0, 50, 40)]),
            ("m2", &[(150, 0, 50, 40)]),
        ],
    );
}

#[test]
fn any_item_at_the_start_of_a_later_line_moves_the_line() {
    // Any item at the start of a later line moves the whole line, not only
    // the first one.
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-wrap:wrap'>\
             <div id=x style='width:100px;height:70px'></div><div id=a style='width:50px'>\
             <div id=a1 style='height:5px'></div></div><div id=b style='width:50px'>\
             <div id=m2 style='contain:size;height:40px'></div></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 40)]),
            ("x", &[(0, 0, 100, 70)]),
            ("a", &[(100, 0, 50, 40)]),
            ("a1", &[(100, 0, 50, 5)]),
            ("b", &[(150, 0, 50, 40)]),
            ("m2", &[(150, 0, 50, 40)]),
        ],
    );
}

#[test]
fn containers_the_moves_do_not_model_still_lay_out() {
    let mono = "<div id=m style='contain:size;height:60px'></div>";
    let pushed = format!("<div id=a style='height:60px'></div><div>{mono}</div>");
    let cases = [
        // An absolutely positioned box, which would not follow the moves.
        format!(
            "<div style='display:flex;flex-direction:column;position:relative'>{pushed}\
             <div style='position:absolute;top:0;height:10px'></div></div>"
        ),
        // A minimum height, a multi-line column container, reversed lines,
        // a grid and a row gap.
        format!("<div style='display:flex;flex-direction:column;min-height:10px'>{pushed}</div>"),
        format!("<div style='display:flex;flex-direction:column;flex-wrap:wrap'>{pushed}</div>"),
        format!("<div style='display:flex;flex-wrap:wrap-reverse'><div style='width:100%'>{pushed}</div></div>"),
        format!("<div style='display:flex'><div style='display:grid'>{pushed}</div></div>"),
        format!("<div style='display:flex;flex-wrap:wrap;row-gap:10px'><div style='width:100%'>{pushed}</div></div>"),
        // Items aligned at their center, and one pushed down by an automatic
        // margin.
        format!("<div style='display:flex;align-items:center'><div>{pushed}</div></div>"),
        format!("<div style='display:flex;align-items:start'><div style='margin-top:auto'>{pushed}</div></div>"),
        // A monolithic box taller than a column, which overflows it, and one
        // that moves past the end of a container with a height: the
        // container is sliced as if they were plain boxes.
        "<div style='display:flex;flex-direction:column'><div style='height:30px'></div>\
         <div><div style='height:10px'></div><div id=m style='contain:size;height:150px'></div></div></div>"
            .to_owned(),
        format!("<div style='display:flex;flex-direction:column;height:120px'>{pushed}</div>"),
        // A forced break between the items as well.
        format!(
            "<div style='display:flex;flex-direction:column'>{pushed}\
             <div style='height:10px;break-before:column'></div></div>"
        ),
    ];
    for content in cases {
        for fill in ["auto", "balance"] {
            let body = columns(fill, &content);
            let got = crate::multicol_flex_forced_break_surface::visible(&body);
            assert!(got.contains_key("m"), "{body}");
        }
    }
}
