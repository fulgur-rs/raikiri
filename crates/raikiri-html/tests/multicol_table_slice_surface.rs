//! A table whose cells hold no lines in a multi-column container is laid
//! out whole and sliced across the columns, each column showing the part of
//! it that falls there, as Chromium breaks its rows and cells at a column
//! end. The values are the ones Chromium gives for the same documents.

use crate::multicol_flex_forced_break_surface::expect;

#[test]
fn rows_of_a_length_are_cut_where_the_filled_columns_end() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:auto\"><table id=f style='border-spacing:0;width:100%'><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(0, 0, 100, 40)]),
            ("r2", &[(0, 40, 100, 40)]),
            ("r3", &[(0, 80, 100, 20), (100, 0, 100, 20)]),
            ("r4", &[(100, 20, 100, 40)]),
        ],
    );
}

#[test]
fn balanced_columns_share_the_rows() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:balance\"><table id=f style='border-spacing:0;width:100%'><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(0, 0, 100, 40)]),
            ("r2", &[(100, 0, 100, 40)]),
            ("r3", &[(200, 0, 100, 40)]),
            ("r4", &[(300, 0, 100, 40)]),
        ],
    );
}

#[test]
fn the_border_of_the_table_is_cut_with_it() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:auto\"><table id=f style='width:100%;border-spacing:0;border:5px solid'><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(5, 5, 90, 40)]),
            ("r2", &[(5, 45, 90, 40)]),
            ("r3", &[(5, 85, 90, 15), (105, 0, 90, 25)]),
            ("r4", &[(105, 25, 90, 40)]),
        ],
    );
}

#[test]
fn balanced_columns_cut_the_border_of_the_table_with_it() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:balance\"><table id=f style='width:100%;border-spacing:0;border:5px solid'><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(5, 5, 90, 38), (105, 0, 90, 3)]),
            ("r2", &[(105, 3, 90, 40)]),
            ("r3", &[(205, 0, 90, 40)]),
            ("r4", &[(205, 40, 90, 3), (305, 0, 90, 38)]),
        ],
    );
}

#[test]
fn rows_below_a_caption_continue_in_the_next_column() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:auto\"><table id=f style='width:100%;border-spacing:0'><caption id=cap style='height:30px'></caption><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(0, 30, 100, 40)]),
            ("r2", &[(0, 70, 100, 30), (100, 0, 100, 10)]),
            ("r3", &[(100, 10, 100, 40)]),
            ("r4", &[(100, 50, 100, 40)]),
        ],
    );
}

#[test]
fn balanced_columns_share_the_rows_below_a_caption() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:balance\"><table id=f style='width:100%;border-spacing:0'><caption id=cap style='height:30px'></caption><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("r1", &[(0, 30, 100, 18), (100, 0, 100, 23)]),
            ("r2", &[(100, 23, 100, 25), (200, 0, 100, 15)]),
            ("r3", &[(200, 15, 100, 33), (300, 0, 100, 8)]),
            ("r4", &[(300, 8, 100, 40)]),
        ],
    );
}

#[test]
fn collapsed_borders_leave_no_spacing_to_drop() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:auto\"><table id=f style='width:100%;border-spacing:0;border-collapse:collapse'><tr id=r1 style='height:40px'><td id=c1></td></tr><tr id=r2 style='height:40px'><td id=c2></td></tr><tr id=r3 style='height:40px'><td id=c3></td></tr><tr id=r4 style='height:40px'><td id=c4></td></tr></table></div>",
        &[
            ("c1", &[(0, 0, 100, 40)]),
            ("c2", &[(0, 40, 100, 40)]),
            ("c3", &[(0, 80, 100, 20), (100, 0, 100, 20)]),
            ("c4", &[(100, 20, 100, 40)]),
        ],
    );
}

#[test]
fn a_row_taller_than_a_column_continues_through_the_others() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:balance\"><table id=f style='width:100%;border-spacing:0'><tr id=r1 style='height:40px'><td id=c1 style='padding:5px'></td></tr><tr id=r2 style='height:140px'><td id=c2></td></tr></table></div>",
        &[
            ("c1", &[(0, 0, 100, 40)]),
            (
                "c2",
                &[
                    (0, 40, 100, 5),
                    (100, 0, 100, 45),
                    (200, 0, 100, 45),
                    (300, 0, 100, 45),
                ],
            ),
        ],
    );
}

#[test]
fn content_at_the_top_of_a_cell_is_cut_with_it() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:balance\"><table id=f style='width:100%;border-spacing:0'><tr style='height:70px'><td style='vertical-align:top'><div id=x1 style='height:30px'></div></td></tr><tr style='height:70px'><td style='vertical-align:top'><div id=x2 style='height:60px'></div></td></tr></table></div>",
        &[
            ("x1", &[(1, 1, 98, 30)]),
            ("x2", &[(201, 1, 98, 34), (301, 0, 98, 26)]),
        ],
    );
}

#[test]
fn a_table_in_a_sliced_flex_container_is_cut_with_it() {
    expect(
        "<div style=\"columns:4;column-gap:0;width:400px;height:100px;column-fill:auto\"><div id=f style='display:flex;flex-direction:column'><table style='width:100%;border-spacing:0'><tr id=r1 style='height:70px'><td></td></tr><tr id=r2 style='height:70px'><td></td></tr></table></div></div>",
        &[
            ("r1", &[(0, 0, 100, 70)]),
            ("r2", &[(0, 70, 100, 30), (100, 0, 100, 40)]),
        ],
    );
}
