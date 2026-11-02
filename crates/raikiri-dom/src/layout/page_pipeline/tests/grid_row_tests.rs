use super::*;

#[derive(Debug, PartialEq)]
struct RowPosition {
    grid: f32,
    items: [f32; 2],
    height: f32,
    after: f32,
    pages: usize,
}

/// A 20px item and a 50px item in two grid columns after a 60px lead, with
/// a 10px block after the grid. An atomic second item holds its text in a
/// 50px inline-block; the item itself still has its stated block size.
fn row(grid_css: &str, second_css: &str, atomic: bool) -> RowPosition {
    row_with(
        60,
        grid_css,
        "",
        second_css,
        "",
        atomic.then_some("inline-block"),
    )
}

fn row_with(
    lead: u32,
    grid_css: &str,
    first_css: &str,
    second_css: &str,
    after_css: &str,
    second_wrapper: Option<&str>,
) -> RowPosition {
    let (mut grid, mut first, mut second, mut after) = (0, 0, 0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        doc.append_element(
            Some(body),
            "div",
            Some(&format!("display:block;height:{lead}px")),
        );
        grid = doc.append_element(
            Some(body),
            "div",
            Some(&format!(
                "display:grid;grid-template-columns:1fr 1fr;font-family:Ahem;font-size:10px;line-height:10px;{grid_css}"
            )),
        );
        first = doc.append_element(
            Some(grid),
            "div",
            Some(&format!("display:block;height:20px;{first_css}")),
        );
        doc.append_text(first, "A");
        second = doc.append_element(
            Some(grid),
            "div",
            Some(&format!("display:block;height:50px;{second_css}")),
        );
        if let Some(display) = second_wrapper {
            let inline = doc.append_element(
                Some(second),
                "span",
                Some(&format!("display:{display};height:50px")),
            );
            doc.append_text(inline, "B");
        } else {
            doc.append_text(second, "B");
        }
        after = doc.append_element(Some(body), "div", Some(&format!("display:block;height:10px;font-family:Ahem;font-size:10px;line-height:10px;{after_css}")));
        doc.append_text(after, "F");
    });
    RowPosition {
        grid: page_flow_y(&doc, grid),
        items: [page_flow_y(&doc, first), page_flow_y(&doc, second)],
        height: doc.nodes[grid].layout.size.height,
        after: page_flow_y(&doc, after),
        pages,
    }
}

#[test]
fn an_avoid_item_moves_its_grid_row_and_the_unpadded_grid_whole() {
    for avoid in ["avoid", "avoid-page"] {
        assert_eq!(
            row("", &format!("break-inside:{avoid}"), false),
            RowPosition {
                grid: 100.0,
                items: [100.0, 100.0],
                height: 50.0,
                after: 150.0,
                pages: 2
            },
            "{avoid}"
        );
    }
}

#[test]
fn an_avoid_grid_row_leaves_the_leading_padding_or_border_on_the_previous_page() {
    for edge in ["padding-top:6px", "border-top:6px solid"] {
        assert_eq!(
            row(edge, "break-inside:avoid", false),
            RowPosition {
                grid: 60.0,
                items: [100.0, 100.0],
                height: 90.0,
                after: 150.0,
                pages: 2
            },
            "{edge}"
        );
    }
}

#[test]
fn an_atomic_inline_inside_an_item_keeps_the_first_grid_row_together() {
    assert_eq!(
        row("", "", true),
        RowPosition {
            grid: 100.0,
            items: [100.0, 100.0],
            height: 50.0,
            after: 150.0,
            pages: 2
        }
    );
}

#[test]
fn cuttable_grid_items_keep_splitting_at_the_page_end() {
    for display in ["", "display:inline-block"] {
        assert_eq!(
            row("", display, false),
            RowPosition {
                grid: 60.0,
                items: [60.0, 60.0],
                height: 50.0,
                after: 110.0,
                pages: 2
            },
            "{display}"
        );
    }
    assert_eq!(
        row("padding-top:6px", "", false),
        RowPosition {
            grid: 60.0,
            items: [66.0, 66.0],
            height: 56.0,
            after: 116.0,
            pages: 2
        }
    );
}

#[test]
fn oversized_avoid_rows_move_once_from_the_middle_and_stay_at_page_start() {
    assert_eq!(
        row_with(60, "", "", "height:150px;break-inside:avoid", "", None),
        RowPosition {
            grid: 100.0,
            items: [100.0, 100.0],
            height: 150.0,
            after: 250.0,
            pages: 3
        }
    );
    assert_eq!(
        row_with(0, "", "", "height:150px;break-inside:avoid", "", None),
        RowPosition {
            grid: 0.0,
            items: [0.0, 0.0],
            height: 150.0,
            after: 150.0,
            pages: 2
        }
    );
}

#[test]
fn padded_grid_rows_can_leave_edges_at_page_start_or_at_the_exact_page_end() {
    assert_eq!(
        row_with(
            0,
            "padding-top:6px",
            "",
            "height:98px;break-inside:avoid",
            "",
            None
        ),
        RowPosition {
            grid: 0.0,
            items: [100.0, 100.0],
            height: 198.0,
            after: 200.0,
            pages: 3
        }
    );
    assert_eq!(
        row_with(94, "padding-top:6px", "", "break-inside:avoid", "", None),
        RowPosition {
            grid: 94.0,
            items: [100.0, 100.0],
            height: 56.0,
            after: 150.0,
            pages: 2
        }
    );
}

#[test]
fn two_avoid_items_do_not_pull_the_grids_leading_edges_to_the_next_page() {
    assert_eq!(
        row_with(
            60,
            "padding-top:6px",
            "height:50px;break-inside:avoid",
            "break-inside:avoid",
            "",
            None
        ),
        RowPosition {
            grid: 60.0,
            items: [100.0, 100.0],
            height: 90.0,
            after: 150.0,
            pages: 2
        }
    );
}

#[test]
fn atomic_grid_items_keep_the_padded_row_together_but_block_children_can_split() {
    assert_eq!(
        row("padding-top:6px", "", true),
        RowPosition {
            grid: 60.0,
            items: [100.0, 100.0],
            height: 90.0,
            after: 150.0,
            pages: 2
        }
    );
    assert_eq!(
        row_with(60, "", "", "", "", Some("block")),
        RowPosition {
            grid: 60.0,
            items: [60.0, 60.0],
            height: 50.0,
            after: 110.0,
            pages: 2
        }
    );
}

#[test]
fn row_moves_preserve_alignment_and_relative_offsets() {
    assert_eq!(
        row_with(60, "", "align-self:end", "break-inside:avoid", "", None),
        RowPosition {
            grid: 100.0,
            items: [130.0, 100.0],
            height: 50.0,
            after: 150.0,
            pages: 2
        }
    );
    assert_eq!(
        row_with(
            60,
            "",
            "",
            "break-inside:avoid;position:relative;top:30px",
            "",
            None
        ),
        RowPosition {
            grid: 100.0,
            items: [100.0, 130.0],
            height: 50.0,
            after: 150.0,
            pages: 2
        }
    );
    assert_eq!(
        row_with(
            60,
            "position:relative;top:10px",
            "",
            "break-inside:avoid",
            "",
            None
        ),
        RowPosition {
            grid: 110.0,
            items: [110.0, 110.0],
            height: 50.0,
            after: 150.0,
            pages: 2
        }
    );
}

#[test]
fn a_row_that_fits_exactly_stays_and_content_growth_keeps_after_box_spacing() {
    assert_eq!(
        row_with(50, "", "", "break-inside:avoid", "", None),
        RowPosition {
            grid: 50.0,
            items: [50.0, 50.0],
            height: 50.0,
            after: 100.0,
            pages: 2
        }
    );
    assert_eq!(
        row_with(
            60,
            "padding-top:6px",
            "",
            "break-inside:avoid",
            "margin-top:10px",
            None
        ),
        RowPosition {
            grid: 60.0,
            items: [100.0, 100.0],
            height: 90.0,
            after: 160.0,
            pages: 2
        }
    );
    assert_eq!(
        row_with(
            60,
            "padding-top:6px",
            "",
            "break-inside:avoid",
            "break-before:page",
            None
        ),
        RowPosition {
            grid: 60.0,
            items: [100.0, 100.0],
            height: 90.0,
            after: 200.0,
            pages: 3
        }
    );
}

fn placed_rows(
    grid_css: &str,
    item_css: &[&str],
    contents: bool,
) -> (f32, Vec<f32>, f32, f32, usize) {
    let (mut grid, mut after) = (0, 0);
    let mut items = Vec::new();
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        grid = doc.append_element(Some(body), "div", Some(&format!(
            "display:grid;grid-template-columns:1fr 1fr;font-family:Ahem;font-size:10px;line-height:10px;{grid_css}"
        )));
        let parent = if contents {
            doc.append_element(Some(grid), "section", Some("display:contents"))
        } else {
            grid
        };
        for css in item_css {
            let item =
                doc.append_element(Some(parent), "div", Some(&format!("display:block;{css}")));
            doc.append_text(item, "A");
            items.push(item);
        }
        after = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:10px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(after, "F");
    });
    (
        page_flow_y(&doc, grid),
        items.iter().map(|&item| page_flow_y(&doc, item)).collect(),
        doc.nodes[grid].layout.size.height,
        page_flow_y(&doc, after),
        pages,
    )
}

#[test]
fn resolved_first_row_membership_and_later_rows_follow_one_content_shift() {
    // The first child belongs to row 2; DOM order cannot identify row 1.
    let items = [
        "height:20px;grid-row:2;grid-column:1",
        "height:20px;grid-row:1;grid-column:1",
        "height:50px;grid-row:1;grid-column:2;break-inside:avoid",
    ];
    assert_eq!(
        placed_rows("row-gap:10px", &items, false),
        (100.0, vec![160.0, 100.0, 100.0], 80.0, 180.0, 2)
    );
    assert_eq!(
        placed_rows("padding-top:6px;row-gap:10px", &items, false),
        (60.0, vec![160.0, 100.0, 100.0], 120.0, 180.0, 2)
    );
}

#[test]
fn contents_and_order_modified_children_use_the_resolved_row_projection() {
    assert_eq!(
        placed_rows(
            "padding-top:6px",
            &["height:20px", "height:50px;break-inside:avoid"],
            true
        ),
        (60.0, vec![100.0, 100.0], 90.0, 150.0, 2)
    );
    // The avoided item is visited first, but order places it in row 2. It
    // must not cause the first row to move as a whole.
    let (grid, items, ..) = placed_rows(
        "",
        &[
            "height:50px;break-inside:avoid;order:2",
            "height:20px;order:1",
            "height:20px;order:0",
        ],
        false,
    );
    assert_eq!(grid, 60.0);
    assert_eq!(&items[1..], &[60.0, 60.0]);
}

#[test]
fn a_grid_after_content_already_reaching_a_later_page_uses_that_pages_boundary() {
    assert_eq!(
        row_with(190, "", "", "break-inside:avoid", "", None),
        RowPosition {
            grid: 200.0,
            items: [200.0, 200.0],
            height: 50.0,
            after: 250.0,
            pages: 3
        }
    );
}

#[test]
fn a_padded_row_after_a_moved_sibling_adds_its_content_delta_once() {
    let (mut grid, mut first, mut second, mut after) = (0, 0, 0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:80px"));
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:50px;break-inside:avoid"),
        );
        grid = doc.append_element(Some(body), "div", Some("display:grid;grid-template-columns:1fr 1fr;padding-top:6px;font-family:Ahem;font-size:10px;line-height:10px"));
        first = doc.append_element(Some(grid), "div", Some("display:block;height:20px"));
        doc.append_text(first, "A");
        second = doc.append_element(
            Some(grid),
            "div",
            Some("display:block;height:50px;break-inside:avoid"),
        );
        doc.append_text(second, "B");
        after = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:10px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(after, "F");
    });
    assert_eq!(
        (
            page_flow_y(&doc, grid),
            page_flow_y(&doc, first),
            page_flow_y(&doc, second),
            doc.nodes[grid].layout.size.height,
            page_flow_y(&doc, after),
            pages
        ),
        (150.0, 200.0, 200.0, 100.0, 250.0, 3)
    );
}

#[test]
fn oversized_padded_rows_are_sent_once_even_when_they_cannot_fit_a_full_page() {
    assert_eq!(
        row_with(
            0,
            "padding-top:6px",
            "",
            "height:150px;break-inside:avoid",
            "",
            None
        ),
        RowPosition {
            grid: 0.0,
            items: [100.0, 100.0],
            height: 250.0,
            after: 250.0,
            pages: 3
        }
    );
}

#[test]
fn fixed_tracks_and_bounded_grid_boxes_keep_their_fit_behavior() {
    for css in ["height:30px", "grid-template-rows:30px"] {
        assert_eq!(
            row_with(
                60,
                css,
                "height:10px",
                "height:20px;break-inside:avoid",
                "",
                None
            ),
            RowPosition {
                grid: 60.0,
                items: [60.0, 60.0],
                height: 30.0,
                after: 90.0,
                pages: 1
            },
            "{css}"
        );
    }
    assert_eq!(
        placed_rows(
            "",
            &[
                "height:10px;grid-row:1 / span 2",
                "height:20px;break-inside:avoid"
            ],
            false
        ),
        (60.0, vec![60.0, 60.0], 20.0, 80.0, 1)
    );
}

#[test]
fn hidden_grid_children_do_not_join_the_row_and_absolute_boxes_keep_their_origin() {
    for hidden in [true, false] {
        let (mut grid, mut first, mut second, mut absolute) = (0, 0, 0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            doc.append_element(Some(body), "div", Some("display:block;height:60px"));
            grid = doc.append_element(Some(body), "div", Some("display:grid;grid-template-columns:1fr 1fr;position:relative;font-family:Ahem;font-size:10px;line-height:10px"));
            first = doc.append_element(Some(grid), "div", Some("display:block;height:10px"));
            doc.append_text(first, "A");
            second = doc.append_element(
                Some(grid),
                "div",
                Some(if hidden {
                    "display:block;height:50px;break-inside:avoid"
                } else {
                    "display:block;height:20px;break-inside:avoid"
                }),
            );
            doc.append_text(second, "B");
            absolute = doc.append_element(
                Some(grid),
                "div",
                Some(if hidden {
                    "display:none;position:absolute;top:3px;height:5px;width:5px"
                } else {
                    "display:block;position:absolute;top:3px;height:5px;width:5px"
                }),
            );
        });
        let top = if hidden { 100.0 } else { 60.0 };
        assert_eq!(
            (
                page_flow_y(&doc, grid),
                page_flow_y(&doc, first),
                page_flow_y(&doc, second)
            ),
            (top, top, top)
        );
        if !hidden {
            assert_eq!(page_flow_y(&doc, absolute), 63.0);
        }
    }
}

#[test]
fn a_nested_padded_grid_grows_its_parent_and_moves_both_following_blocks_once() {
    let (mut parent, mut grid, mut first, mut second, mut inside, mut outside) = (0, 0, 0, 0, 0, 0);
    let (doc, pages) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        parent = doc.append_element(Some(body), "div", Some("display:block;padding-top:5px"));
        doc.append_element(Some(parent), "div", Some("display:block;height:10px"));
        grid = doc.append_element(Some(parent), "div", Some("display:grid;grid-template-columns:1fr 1fr;padding-top:6px;font-family:Ahem;font-size:10px;line-height:10px"));
        first = doc.append_element(Some(grid), "div", Some("display:block;height:20px"));
        doc.append_text(first, "A");
        second = doc.append_element(
            Some(grid),
            "div",
            Some("display:block;height:50px;break-inside:avoid"),
        );
        doc.append_text(second, "B");
        inside = doc.append_element(
            Some(parent),
            "div",
            Some("display:block;height:10px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(inside, "F");
        outside = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:10px;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(outside, "G");
    });
    assert_eq!(
        (
            page_flow_y(&doc, parent),
            page_flow_y(&doc, grid),
            page_flow_y(&doc, first),
            page_flow_y(&doc, second)
        ),
        (60.0, 75.0, 100.0, 100.0)
    );
    assert_eq!(
        (
            doc.nodes[parent].layout.size.height,
            doc.nodes[grid].layout.size.height,
            page_flow_y(&doc, inside),
            page_flow_y(&doc, outside),
            pages
        ),
        (100.0, 75.0, 150.0, 160.0, 2)
    );
}

#[test]
fn repaginating_the_same_document_does_not_accumulate_grid_content_shifts() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_element(Some(body), "div", Some("display:block;height:60px"));
    let grid = doc.append_element(
        Some(body),
        "div",
        Some("display:grid;grid-template-columns:1fr 1fr;padding-top:6px"),
    );
    let first = doc.append_element(Some(grid), "div", Some("display:block;height:20px"));
    doc.append_text(first, "A");
    let second = doc.append_element(
        Some(grid),
        "div",
        Some("display:block;height:50px;break-inside:avoid"),
    );
    doc.append_text(second, "B");
    let after = doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    doc.append_text(after, "F");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    crate::layout::test_support::with_ahem(&mut doc);
    for pass in 0..2 {
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        let pages = layout_pages(&mut doc, &cascade, page).expect("layout");
        assert_eq!(
            RowPosition {
                grid: page_flow_y(&doc, grid),
                items: [page_flow_y(&doc, first), page_flow_y(&doc, second)],
                height: doc.nodes[grid].layout.size.height,
                after: page_flow_y(&doc, after),
                pages: pages.len(),
            },
            RowPosition {
                grid: 60.0,
                items: [100.0, 100.0],
                height: 90.0,
                after: 150.0,
                pages: 2
            },
            "pass {pass}"
        );
    }
}

#[test]
fn an_avoid_item_aligned_late_in_a_tall_row_still_moves_the_first_row_once() {
    let result = row_with(
        60,
        "",
        "height:150px",
        "height:20px;align-self:end;break-inside:avoid",
        "",
        None,
    );
    // This pins the initial row transition; later fragmentation of a very
    // tall aligned row has additional rules beyond this first-row pass.
    assert_eq!(result.grid, 100.0);
    assert_eq!(result.items[0], 100.0);
}

#[test]
fn native_leading_gaps_and_mixed_atomic_blocks_keep_the_existing_pagination_path() {
    use crate::layout::context::LayoutSession;
    use crate::layout::layout_single_page;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    for leading_gap in [true, false] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0;width:100px;font-family:Ahem;font-size:10px;line-height:10px"));
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let grid = doc.append_element(
            Some(body),
            "div",
            Some(if leading_gap {
                "display:grid;grid-template-columns:1fr 1fr;row-gap:10px"
            } else {
                "display:grid;grid-template-columns:1fr 1fr"
            }),
        );
        let first = doc.append_element(
            Some(grid),
            "div",
            Some(if leading_gap {
                "display:block;height:10px;grid-row:2;grid-column:1"
            } else {
                "display:block;height:10px"
            }),
        );
        doc.append_text(first, "A");
        let second = doc.append_element(
            Some(grid),
            "div",
            Some(if leading_gap {
                "display:block;height:50px;break-inside:avoid;grid-row:2;grid-column:2"
            } else {
                "display:block;height:50px"
            }),
        );
        if leading_gap {
            doc.append_text(second, "B");
        } else {
            doc.append_element(Some(second), "div", Some("display:block;height:10px"));
            let atomic = doc.append_element(
                Some(second),
                "span",
                Some("display:inline-block;height:50px"),
            );
            doc.append_text(atomic, "B");
        }
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        crate::layout::test_support::with_ahem(&mut doc);
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_single_page(&mut doc, &cascade, page).expect("layout");
        let tree = LayoutSession::resumed(&mut doc, &cascade);
        assert_eq!(tree.nodes[grid].grid_item_rows.len(), 2);
        if leading_gap {
            assert_eq!(tree.nodes[grid].grid_item_rows[0].block_start, 10.0);
            assert_eq!(
                tree.nodes[grid].layout.padding.top + tree.nodes[grid].layout.border.top,
                0.0
            );
        } else {
            // The atomic line is real, but a block before it makes this a
            // mixed paragraph whose fragmentation stays on the existing path.
            assert!(
                super::super::unplaced_line(&tree, &cascade, second)
                    .is_some_and(|line| line.atomic)
            );
        }
        assert!(
            super::super::grid_rows::first_row(&tree, &cascade, body, grid).is_none(),
            "leading gap: {leading_gap}"
        );
    }
}
