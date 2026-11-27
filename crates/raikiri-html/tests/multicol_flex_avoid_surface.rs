//! Breaks that boxes avoid inside a flex container in a multi-column
//! container, and the space that a break truncates between its items (CSS
//! Fragmentation 3, 3.1, 4.4 and 5.2; CSS Box Alignment 3, 8.1). The values
//! are the ones Chromium gives for the same documents.

use crate::multicol_flex_forced_break_surface::{columns, expect};

fn column_flex(style: &str, items: &str) -> String {
    columns(
        "auto",
        &format!("<div id=f style='display:flex;flex-direction:column;{style}'>{items}</div>"),
    )
}

#[test]
fn an_avoided_break_between_items_moves_to_the_break_before_the_item_above() {
    // The column ends between b and c, where c avoids a break before it, so
    // the break moves before b. A break after b that b avoids does the same.
    for (b, c) in [("", "break-before:avoid"), ("break-after:avoid", "")] {
        expect(
            &column_flex(
                "",
                &format!(
                    "<div id=a style='height:50px'></div><div id=b style='height:50px;{b}'></div>\
                     <div id=c style='height:300px;{c}'></div>"
                ),
            ),
            &[
                ("a", &[(0, 0, 100, 50)]),
                ("b", &[(100, 0, 100, 50)]),
                (
                    "c",
                    &[
                        (100, 50, 100, 50),
                        (200, 0, 100, 100),
                        (300, 0, 100, 100),
                        (400, 0, 100, 50),
                    ],
                ),
            ],
        );
    }
}

#[test]
fn a_chain_of_avoided_breaks_moves_its_boxes_together() {
    expect(
        &column_flex(
            "",
            "<div id=a style='height:40px'></div><div id=b style='height:40px;break-after:avoid'></div>\
             <div id=c style='height:20px;break-after:avoid'></div><div id=d style='height:40px'></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 40)]),
            ("b", &[(100, 0, 100, 40)]),
            ("c", &[(100, 40, 100, 20)]),
            ("d", &[(100, 60, 100, 40)]),
        ],
    );
}

#[test]
fn an_avoided_break_stays_where_the_box_above_starts_its_column() {
    // a fills its column from the top, so the break before b stays.
    expect(
        &column_flex(
            "",
            "<div id=a style='height:100px'></div><div id=b style='height:30px;break-before:avoid'></div>\
             <div id=c style='height:30px'></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 100)]),
            ("b", &[(100, 0, 100, 30)]),
            ("c", &[(100, 30, 100, 30)]),
        ],
    );
}

#[test]
fn a_box_that_avoids_breaks_inside_moves_to_the_next_column() {
    // It moves whole where the next column holds it, with the boxes it holds
    // and with its margin before it truncated.
    for item in [
        "<div id=b style='height:60px;break-inside:avoid'></div>",
        "<div id=b style='break-inside:avoid'><div id=b1 style='height:30px'></div>\
         <div id=b2 style='height:30px'></div></div>",
        "<div id=b style='height:60px;break-inside:avoid;margin-top:5px'></div>",
    ] {
        expect(
            &column_flex(
                "",
                &format!(
                    "<div id=a style='height:50px'></div>{item}<div id=c style='height:30px'></div>"
                ),
            ),
            &[
                ("f", &[(0, 0, 100, 100), (100, 0, 100, 90)]),
                ("a", &[(0, 0, 100, 50)]),
                ("b", &[(100, 0, 100, 60)]),
                ("c", &[(100, 60, 100, 30)]),
            ],
        );
    }
    // One taller than a column moves too, and is cut there.
    expect(
        &column_flex(
            "",
            "<div id=a style='height:50px'></div><div id=b style='height:150px;break-inside:avoid'></div>\
             <div id=c style='height:30px'></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 50)]),
            ("b", &[(100, 0, 100, 100), (200, 0, 100, 50)]),
            ("c", &[(200, 50, 100, 30)]),
        ],
    );
}

#[test]
fn a_break_truncates_the_gap_and_the_margin_after_the_item_before_it() {
    // The column ends in the row gap, or in the block-end margin of a: b
    // starts the next column.
    for (style, a) in [("row-gap:20px", ""), ("", "margin-bottom:20px")] {
        expect(
            &column_flex(
                style,
                &format!(
                    "<div id=a style='height:90px;{a}'></div><div id=b style='height:90px'></div>\
                     <div id=c style='height:5px'></div>"
                ),
            ),
            &[("a", &[(0, 0, 100, 90)]), ("b", &[(100, 0, 100, 90)])],
        );
    }
    // Where the break is avoided and the box before it starts its column,
    // the gap is truncated all the same.
    expect(
        &column_flex(
            "row-gap:20px",
            "<div id=a style='height:90px'></div><div id=b style='height:30px;break-before:avoid'></div>\
             <div id=c style='height:30px'></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 90)]),
            ("b", &[(100, 0, 100, 30)]),
            ("c", &[(100, 50, 100, 30)]),
        ],
    );
}

#[test]
fn a_break_keeps_the_block_start_margin_of_the_item_after_it() {
    // The margin box of b starts the next column, its own margin kept; one
    // that the column end cuts goes on in the next column.
    for (a, b, top) in [
        ("margin-bottom:20px", "margin-top:10px", 10),
        ("", "margin-top:20px", 10),
        ("margin-bottom:5px", "margin-top:20px", 15),
    ] {
        expect(
            &column_flex(
                "",
                &format!(
                    "<div id=a style='height:90px;{a}'></div><div id=b style='height:30px;{b}'></div>"
                ),
            ),
            &[("a", &[(0, 0, 100, 90)]), ("b", &[(100, top, 100, 30)])],
        );
    }
}

#[test]
fn balanced_columns_grow_by_the_least_space_a_moved_box_needs() {
    // The columns balance at 67.5px, where b moves; they grow by the 42.5px
    // b needs, and the content then fits in three columns.
    expect(
        "<div style='columns:4;column-gap:0;width:400px'>\
         <div id=f style='display:flex;flex-direction:column'><div id=a style='height:50px'></div>\
         <div id=b style='height:60px;break-inside:avoid'></div><div id=c style='height:50px'></div>\
         <div id=d style='height:70px;break-inside:avoid'></div><div id=e style='height:40px'></div>\
         </div></div>",
        &[
            (
                "f",
                &[(0, 0, 100, 110), (100, 0, 100, 110), (200, 0, 100, 110)],
            ),
            ("a", &[(0, 0, 100, 50)]),
            ("b", &[(0, 50, 100, 60)]),
            ("c", &[(100, 0, 100, 50)]),
            ("d", &[(200, 0, 100, 70)]),
            ("e", &[(200, 70, 100, 40)]),
        ],
    );
    expect(
        "<div style='columns:3;column-gap:0;width:300px'>\
         <div id=f style='display:flex;flex-direction:column'>\
         <div id=a style='height:40px;break-inside:avoid'></div>\
         <div id=b style='height:40px;break-inside:avoid'></div>\
         <div id=c style='height:40px;break-inside:avoid'></div>\
         <div id=d style='height:40px;break-inside:avoid'></div>\
         <div id=e style='height:40px;break-inside:avoid'></div></div></div>",
        &[
            (
                "f",
                &[(0, 0, 100, 80), (100, 0, 100, 80), (200, 0, 100, 40)],
            ),
            ("c", &[(100, 0, 100, 40)]),
            ("e", &[(200, 0, 100, 40)]),
        ],
    );
}

#[test]
fn absolutely_positioned_children_of_the_columns_leave_them() {
    // The containing block of x is the multi-column container, which places
    // it apart from the columns; its static position is after the flex
    // container, in the overflow column.
    expect(
        "<div style='columns:4;column-gap:0;width:400px;height:100px;column-fill:auto;position:relative'>\
         <div id=f style='display:flex;flex-direction:column'><div id=a style='height:50px'></div>\
         <div id=b style='height:50px'></div><div id=c style='height:300px;break-before:avoid'></div></div>\
         <div id=x style='position:absolute;top:0;width:25px;height:50px'></div>\
         <div id=y style='position:absolute;top:50px;left:0;width:25px;height:50px'></div></div>",
        &[
            ("a", &[(0, 0, 100, 50)]),
            ("b", &[(100, 0, 100, 50)]),
            ("x", &[(400, 0, 25, 50)]),
            ("y", &[(0, 50, 25, 50)]),
        ],
    );
}

#[test]
fn a_box_whose_margin_box_starts_its_column_stays_there() {
    // The break truncates the gap before b, whose margin box then starts the
    // column; b avoids breaks inside it but does not fit, and stays.
    expect(
        &column_flex(
            "row-gap:20px",
            "<div id=a style='height:90px'></div>\
             <div id=b style='height:95px;margin-top:10px;break-inside:avoid'></div>\
             <div id=c style='height:10px'></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 90)]),
            ("b", &[(100, 10, 100, 90), (200, 0, 100, 5)]),
            ("c", &[(200, 25, 100, 10)]),
        ],
    );
}

#[test]
fn an_avoided_break_before_the_second_item_moves_the_container() {
    // The break before c moves to the one before b, the first item, which
    // is the break before the flex container.
    expect(
        &columns(
            "auto",
            "<div id=z style='height:30px'></div>\
             <div id=f style='display:flex;flex-direction:column'><div id=b style='height:70px'></div>\
             <div id=c style='height:300px;break-before:avoid'></div></div>",
        ),
        &[
            ("z", &[(0, 0, 100, 30)]),
            ("b", &[(100, 0, 100, 70)]),
            (
                "c",
                &[
                    (100, 70, 100, 30),
                    (200, 0, 100, 100),
                    (300, 0, 100, 100),
                    (400, 0, 100, 70),
                ],
            ),
        ],
    );
}

#[test]
fn a_break_keeps_the_free_space_the_container_distributes() {
    // Only the row gap is truncated: the space that justify-content puts
    // between the items goes on in the next column.
    expect(
        &column_flex(
            "height:300px;justify-content:space-between;row-gap:10px",
            "<div id=a style='height:50px'></div><div id=b style='height:50px'></div>\
             <div id=c style='height:50px'></div><div id=d style='height:50px'></div>",
        ),
        &[("a", &[(0, 0, 100, 50)]), ("d", &[(200, 50, 100, 50)])],
    );
}

#[test]
fn balanced_columns_grow_by_the_space_an_avoided_break_moves() {
    // At 45px the column ends between b and c, which b avoids, so b moves
    // down by 25px; the columns grow by as much and the break falls in c.
    expect(
        "<div style='columns:2;column-gap:0;width:200px'>\
         <div id=f style='display:flex;flex-direction:column'><div id=a style='height:20px'></div>\
         <div id=b style='height:25px;break-after:avoid'></div>\
         <div id=c style='height:45px'></div></div></div>",
        &[
            ("f", &[(0, 0, 100, 70), (100, 0, 100, 20)]),
            ("b", &[(0, 20, 100, 25)]),
            ("c", &[(0, 45, 100, 25), (100, 0, 100, 20)]),
        ],
    );
}

#[test]
fn a_box_that_avoids_breaks_moves_with_the_monolithic_box_it_holds() {
    expect(
        &column_flex(
            "",
            "<div id=a style='height:50px'></div><div id=w style='break-inside:avoid'>\
             <div id=b style='height:30px;contain:size'></div><div id=c style='height:30px'></div>\
             </div><div id=d style='height:30px'></div>",
        ),
        &[
            ("w", &[(100, 0, 100, 60)]),
            ("b", &[(100, 0, 100, 30)]),
            ("c", &[(100, 30, 100, 30)]),
            ("d", &[(100, 60, 100, 30)]),
        ],
    );
}

#[test]
fn a_gap_longer_than_a_column_is_truncated_at_the_first_column_end() {
    // The gap crosses two column ends; the break falls at the first, so b
    // starts the next column, its own block-start margin kept.
    for (a, margin) in [("contain:size", 0), ("", 10)] {
        expect(
            &column_flex(
                "row-gap:220px",
                &format!(
                    "<div id=a style='height:90px;{a}'></div>\
                     <div id=b style='height:90px;margin-top:{margin}px'></div>"
                ),
            ),
            &[("a", &[(0, 0, 100, 90)]), ("b", &[(100, margin, 100, 90)])],
        );
    }
}

#[test]
fn a_percentage_gap_resolves_against_the_block_size_of_the_container() {
    // The gap is 22px, 10% of 220px, and is truncated at the column end.
    expect(
        &column_flex(
            "height:220px;row-gap:10%",
            "<div id=a style='height:90px;contain:size'></div><div id=b style='height:90px'></div>",
        ),
        &[
            (
                "f",
                &[(0, 0, 100, 100), (100, 0, 100, 100), (200, 0, 100, 20)],
            ),
            ("a", &[(0, 0, 100, 90)]),
            ("b", &[(100, 0, 100, 90)]),
        ],
    );
}
