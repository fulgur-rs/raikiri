//! A grid container whose items hold no lines in a multi-column container is
//! laid out whole and sliced across the columns, each column showing the
//! part of it that falls there, as Chromium breaks its rows at a column end.
//! The values are the ones Chromium gives for the same documents.

use crate::multicol_flex_forced_break_surface::{columns, expect};

#[test]
fn a_grid_taller_than_its_columns_continues_in_the_next_ones() {
    expect(
        &columns(
            "balance",
            "<div id=f style='display:grid'><div id=a style='height:150px'></div></div>",
        ),
        &[(
            "a",
            &[
                (0, 0, 100, 38),
                (100, 0, 100, 38),
                (200, 0, 100, 38),
                (300, 0, 100, 38),
            ],
        )],
    );
}

#[test]
fn explicit_rows_are_cut_where_the_columns_end() {
    expect(
        &columns(
            "balance",
            "<div id=f style='display:grid;grid-template-rows:30px 30px 30px'><div id=a></div><div id=b></div><div id=c></div></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 23), (100, 0, 100, 8)]),
            ("b", &[(100, 8, 100, 15), (200, 0, 100, 15)]),
            ("c", &[(200, 15, 100, 8), (300, 0, 100, 23)]),
        ],
    );
}

#[test]
fn items_side_by_side_are_cut_at_the_same_column_end() {
    expect(
        &columns(
            "balance",
            "<div id=f style='display:grid;grid-template-columns:1fr 1fr'><div id=a style='height:30px'></div>\
             <div id=b style='height:20px'></div><div id=c style='height:30px'></div><div id=d style='height:40px'></div></div>",
        ),
        &[
            ("a", &[(0, 0, 50, 18), (100, 0, 50, 13)]),
            ("b", &[(50, 0, 50, 18), (150, 0, 50, 3)]),
            ("c", &[(100, 13, 50, 5), (200, 0, 50, 18), (300, 0, 50, 8)]),
            ("d", &[(150, 13, 50, 5), (250, 0, 50, 18), (350, 0, 50, 18)]),
        ],
    );
}

