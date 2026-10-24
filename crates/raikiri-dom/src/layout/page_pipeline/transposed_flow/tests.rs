use super::*;
use crate::layout::protocol::{LayoutInput, RequestedAxis, RunMode, SizingMode};
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use raikiri_layout_geometry::Line;
use raikiri_layout_style::values::{Dimension, IntoEdges, LengthPercentageAuto};

/// Paginate an 800x600 page holding a root styled `html_css` and a body
/// styled `body_css` (Ahem text, 10px lines) with one block child per entry
/// of `children`: `(tag, css, text)`. Returns the document, the body and its
/// children, and the pages.
fn paginate(
    html_css: &str,
    body_css: &str,
    children: &[(&str, &str, &str)],
) -> (Document, Vec<usize>, Vec<PageSlice>) {
    let (doc, _, ids, pages) = paginate_with_cascade(html_css, body_css, children);
    (doc, ids, pages)
}

/// [`paginate`], also returning the cascade.
fn paginate_with_cascade(
    html_css: &str,
    body_css: &str,
    children: &[(&str, &str, &str)],
) -> (
    Document,
    raikiri_style::CascadeResult,
    Vec<usize>,
    Vec<PageSlice>,
) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Some(format!("display:block;{html_css}").as_str()),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Some(
            format!("display:block;font-family:Ahem;font-size:10px;line-height:10px;{body_css}")
                .as_str(),
        ),
    );
    let mut ids = vec![body];
    for &(tag, css, text) in children {
        let child = doc.append_element(
            Some(body),
            tag,
            Some(format!("display:block;{css}").as_str()),
        );
        if !text.is_empty() {
            doc.append_text(child, text);
        }
        ids.push(child);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let pages = layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("pages");
    (doc, cascade, ids, pages)
}

fn origins(pages: &[PageSlice]) -> Vec<f32> {
    pages.iter().map(|page| page.content_origin_y).collect()
}

#[test]
fn vertical_rl_pages_continue_to_the_left() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            ("div", "width:800px", ""),
            ("div", "width:800px", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0, 1600.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-800.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[3]), (-1600.0, 0.0, 800.0, 600.0));
}

#[test]
fn vertical_lr_pages_continue_to_the_right() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-lr",
        "margin:0",
        &[("div", "width:800px", ""), ("div", "width:800px", "")],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (800.0, 0.0, 800.0, 600.0));
}

#[test]
fn a_block_that_avoids_breaks_moves_to_the_next_page() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:600px", ""),
            ("div", "width:700px;break-inside:avoid", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (200.0, 0.0, 600.0, 600.0));
    // The second block starts the second page, whose block-start edge is
    // 800px left of the first page's.
    assert_eq!(absolute_rect(&doc, ids[2]), (-700.0, 0.0, 700.0, 600.0));
}

#[test]
fn the_block_end_margin_of_the_body_does_not_add_a_page() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0 20px",
        &[
            ("div", "width:600px", ""),
            ("div", "width:800px;break-inside:avoid", ""),
        ],
    );
    // The second block fills the second page; the body's block-end margin
    // after it is truncated at the end of the flow.
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (180.0, 0.0, 600.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-800.0, 0.0, 800.0, 600.0));
    // The body's margins stay outside its border box on physical axes.
    let layout = doc.nodes[ids[0]].layout;
    assert_eq!((layout.margin.left, layout.margin.right), (20.0, 20.0));
    assert_eq!((layout.padding.left, layout.padding.right), (0.0, 0.0));
}

#[test]
fn sideways_rl_pages_continue_to_the_left() {
    let (doc, ids, pages) = paginate(
        "writing-mode:sideways-rl",
        "margin:0",
        &[("div", "width:800px", ""), ("div", "width:800px", "")],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-800.0, 0.0, 800.0, 600.0));
}

#[test]
fn sideways_lr_blocks_start_at_the_bottom_and_pages_continue_to_the_right() {
    let (doc, ids, pages) = paginate(
        "writing-mode:sideways-lr",
        "margin:0",
        &[
            ("div", "width:800px;height:100px", ""),
            ("div", "width:800px;height:100px", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    // The inline axis runs from bottom to top.
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 500.0, 800.0, 100.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (800.0, 500.0, 800.0, 100.0));
}

#[test]
fn upright_text_keeps_the_direction_of_a_sideways_root() {
    // The sideways modes are horizontal typographic modes, so `upright`
    // leaves `rtl` in place: the inline axis of `sideways-lr` then runs from
    // top to bottom.
    let (doc, ids, _) = paginate(
        "writing-mode:sideways-lr;direction:rtl;text-orientation:upright",
        "margin:0",
        &[("div", "width:800px;height:100px", "")],
    );
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 100.0));
}

#[test]
fn logical_sizes_of_a_sideways_box_map_onto_its_vertical_axes() {
    let (doc, ids, _) = paginate(
        "writing-mode:sideways-rl",
        "margin:0",
        &[("div", "inline-size:100px;block-size:200px", "")],
    );
    assert_eq!(absolute_rect(&doc, ids[1]), (600.0, 0.0, 200.0, 100.0));
}

#[test]
fn a_forced_break_starts_a_page_along_the_block_axis() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:100px", ""),
            ("div", "width:100px;break-before:page", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (700.0, 0.0, 100.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-100.0, 0.0, 100.0, 600.0));
}

#[test]
fn lines_of_a_vertical_paragraph_continue_on_the_next_page() {
    // 100 lines of 10px: the first 80 fill the first page.
    let text = vec!["X"; 100].join(" ");
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("p", "margin:0;height:10px", text.as_str())],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (-200.0, 0.0, 1000.0, 10.0));
}

#[test]
fn a_horizontal_root_is_paginated_along_y() {
    let (_, _, pages) = paginate(
        "",
        "margin:0",
        &[("div", "height:600px", ""), ("div", "height:600px", "")],
    );
    assert_eq!(origins(&pages), [0.0, 600.0]);
}

#[test]
fn a_moved_block_of_a_vertical_lr_root_lands_on_the_next_page() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-lr",
        "margin:0",
        &[
            ("div", "width:600px", ""),
            ("div", "width:700px;break-inside:avoid", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 600.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (800.0, 0.0, 700.0, 600.0));
}

#[test]
fn logical_sizes_of_a_vertical_box_keep_their_axes() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "block-size:100px;inline-size:300px", "")],
    );
    assert_eq!(origins(&pages), [0.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (700.0, 0.0, 100.0, 300.0));
}

#[test]
fn a_table_of_bare_text_stays_at_the_block_start_edge() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "display:table", "X")],
    );
    assert_eq!(origins(&pages), [0.0]);
    assert_eq!(doc.anonymous_table_cells(ids[1]).count(), 1);
    let (x, y, width, _) = absolute_rect(&doc, ids[1]);
    assert_eq!((x + width, y), (800.0, 0.0));
}

/// The border boxes `node` has on each page after projection, page-local.
fn projected_rects(
    doc: &mut Document,
    cascade: &raikiri_style::CascadeResult,
    pages: &[PageSlice],
    node: usize,
) -> Vec<Vec<(f32, f32, f32, f32)>> {
    let page = page_box_800x600();
    let geometry = (
        page,
        crate::layout::page_margins(cascade, page),
        crate::layout::page_content_insets(cascade, page),
    );
    doc.project_pages(cascade, page, pages, &vec![geometry; pages.len()])
        .expect("projection");
    pages
        .iter()
        .map(|slice| {
            doc.page_fragments(slice.page_index)
                .filter(|fragment| fragment.node() == raikiri_traits::NodeId::new(node as u64))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.x, rect.y, rect.width, rect.height)
                })
                .collect()
        })
        .collect()
}

#[test]
fn projection_places_blocks_on_their_pages_along_x() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "width:800px", ""), ("div", "width:800px", "")],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[1]),
        [vec![(0.0, 0.0, 800.0, 600.0)], vec![]]
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(0.0, 0.0, 800.0, 600.0)]]
    );
}

#[test]
fn projection_splits_a_vertical_paragraph_by_its_lines() {
    // 100 lines of 10px: the first 80 fill the first page.
    let text = vec!["X"; 100].join(" ");
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("p", "margin:0;height:10px", text.as_str())],
    );
    let text_node = doc.nodes[ids[1]].children[0];
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, text_node),
        [
            vec![(0.0, 0.0, 800.0, 10.0)],
            vec![(600.0, 0.0, 200.0, 10.0)]
        ]
    );
    let line_ranges: Vec<Vec<_>> = pages
        .iter()
        .map(|slice| {
            doc.page_fragments(slice.page_index)
                .filter(|fragment| fragment.node() == raikiri_traits::NodeId::new(text_node as u64))
                .filter_map(|fragment| fragment.line_range())
                .collect()
        })
        .collect();
    assert_eq!(line_ranges, [0..80, 80..100].map(|range| vec![range]));
}

#[test]
fn projection_places_the_text_of_a_horizontal_box_on_its_page() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            (
                "div",
                "writing-mode:horizontal-tb;width:100px;height:600px",
                "X",
            ),
        ],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(700.0, 0.0, 100.0, 600.0)]]
    );
    assert!(doc.page_text_runs(&cascade, 0).is_empty());
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(runs.len(), 1);
    let (x, y) = runs[0].origin;
    assert!((700.0..800.0).contains(&x), "{x}");
    assert!((0.0..20.0).contains(&y), "{y}");
}

#[test]
fn projection_places_blocks_of_a_vertical_lr_root_on_their_pages() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-lr",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            (
                "div",
                "writing-mode:horizontal-tb;width:780px;height:590px;padding:10px 0 0 20px",
                "X",
            ),
        ],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(0.0, 0.0, 800.0, 600.0)]]
    );
    let content = doc
        .page_fragments(pages[1].page_index)
        .find(|fragment| fragment.node() == raikiri_traits::NodeId::new(ids[2] as u64))
        .and_then(|fragment| fragment.content_rect())
        .map(|rect| (rect.x, rect.y, rect.width, rect.height));
    assert_eq!(content, Some((20.0, 10.0, 780.0, 590.0)));
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(runs.len(), 1);
    assert!(
        (20.0..40.0).contains(&runs[0].origin.0),
        "{:?}",
        runs[0].origin
    );
}

#[test]
fn projection_holds_the_slice_of_a_narrower_page_against_its_right_edge() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            (
                "div",
                "writing-mode:horizontal-tb;width:100px;height:600px",
                "X",
            ),
        ],
    );
    let wide = page_box_800x600();
    let mut narrow = wide;
    narrow.width = 600.0;
    let geometries: Vec<_> = [wide, narrow]
        .into_iter()
        .map(|page| {
            (
                page,
                crate::layout::page_margins(&cascade, page),
                crate::layout::page_content_insets(&cascade, page),
            )
        })
        .collect();
    doc.project_pages(&cascade, wide, &pages, &geometries)
        .expect("projection");
    let rect = doc
        .page_fragments(pages[1].page_index)
        .find(|fragment| fragment.node() == raikiri_traits::NodeId::new(ids[2] as u64))
        .map(|fragment| {
            let rect = fragment.rect();
            (rect.x, rect.y, rect.width, rect.height)
        });
    assert_eq!(rect, Some((500.0, 0.0, 100.0, 600.0)));
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(runs.len(), 1);
    assert!(
        (500.0..600.0).contains(&runs[0].origin.0),
        "{:?}",
        runs[0].origin
    );
}

#[test]
fn continuation_offsets_of_pages_along_x_are_along_the_flow() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:600px", ""),
            ("div", "width:400px", ""),
            ("div", "width:100px", ""),
        ],
    );
    projected_rects(&mut doc, &cascade, &pages, ids[1]);
    let crossing = doc
        .crossing_boxes(pages[1].page_index)
        .into_iter()
        .find(|crossing| crossing.node == raikiri_traits::NodeId::new(ids[2] as u64))
        .map(|crossing| crossing.top);
    assert_eq!(crossing, Some(-200.0));
    assert_eq!(
        doc.page_token_offset(pages[1].page_index, crate::PageStartToken::Node(ids[3])),
        Some(200.0)
    );
}

#[test]
fn pagination_axis_changes_discard_cached_table_preparation() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "html", Some("display:block"));
    let table = doc.append_element(
        Some(root),
        "table",
        Some("display:table;border-collapse:collapse"),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "td", Some("display:table-cell;width:30px;height:10px;border-left:7px solid red;border-top:2px solid blue"));
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::bridged(&mut doc, &cascade);
    let input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size::NONE,
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let layout = |session: &mut LayoutSession<'_>| {
        crate::layout::table::compute_table_layout(session, table.into(), input)
    };
    crate::layout::table::GRID_BUILDS.with(|count| count.set(0));
    let original = layout(&mut session).size;
    assert_eq!(
        crate::layout::table::GRID_BUILDS.with(|count| count.get()),
        1
    );
    let physical_cell = session.nodes[cell].layout.size;
    TransposedDocument::with_transposed(
        &mut session,
        root,
        FlowAxes {
            right_to_left: false,
        },
        800.0,
        |session, _| {
            assert!(session.context.pagination_on_transposed_axes);
            // Pagination consumes geometry; native layout cannot re-enter here.
            let rejected =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout(session)));
            assert!(rejected.is_err());
        },
    );
    assert_eq!(layout(&mut session).size, original);
    assert_eq!(
        crate::layout::table::GRID_BUILDS.with(|count| count.get()),
        2
    );
    assert_eq!(session.nodes[cell].layout.size, physical_cell);
}

#[test]
fn pagination_queries_and_updates_are_rejected() {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
    use raikiri_style::ComputedLengthPercentageOrAuto as Length;
    use taffy::{
        LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer,
        LayoutPartialTree,
    };
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "html",
        Some("display:block;width:10px;height:20px"),
    );
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
    session.context.pagination_on_transposed_axes = true;
    type Attempt = Box<dyn Fn(&mut LayoutSession<'_>)>;
    let attempts: Vec<Attempt> = vec![
        Box::new(move |s| {
            let _ = s.get_core_container_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_block_container_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_block_child_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_oof_item_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_flexbox_container_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_flexbox_child_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_grid_container_style(root.into());
        }),
        Box::new(move |s| {
            let _ = s.get_grid_child_style(root.into());
        }),
        Box::new(move |s| {
            s.record_pass_override(
                root,
                OwnedPropertyOverride::length(LengthProperty::Width, Length::Px(99.0), false)
                    .unwrap(),
            )
        }),
        Box::new(|s| s.push_overrides(Vec::new())),
        Box::new(move |s| {
            s.begin_edge_recording(root, 100.0);
        }),
        Box::new(|s| s.prepare_style_views()),
        Box::new(|s| s.begin_style_bridge()),
        Box::new(move |s| {
            s.compute_child_layout(
                root.into(),
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Horizontal,
                    known_dimensions: Size::NONE,
                    parent_size: Size::NONE,
                    available_space: Size {
                        width: AvailableSpace::MaxContent,
                        height: AvailableSpace::MaxContent,
                    },
                    known_dimensions_are_definite: Size {
                        width: false,
                        height: false,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            );
        }),
    ];
    for (i, attempt) in attempts.into_iter().enumerate() {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| attempt(&mut session)))
                .is_err(),
            "pagination operation {i} was accepted"
        );
    }
}

#[test]
fn pagination_error_and_panic_restore_binding() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "html",
        Some("display:block;width:10px;height:20px"),
    );
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut session = LayoutSession::bridged(&mut doc, &cascade);
    let size = session.style_view(root).size();
    let axes = FlowAxes {
        right_to_left: true,
    };
    let result: Result<(), u8> =
        TransposedDocument::with_transposed(&mut session, root, axes, 800.0, |s, _| {
            assert!(s.context.pagination_on_transposed_axes);
            // The paginator's height is the physical width.
            assert_eq!(s.block_axis(root).height(), size.width);
            Err(19)
        });
    assert_eq!(result, Err(19));
    assert_eq!(session.style_view(root).size(), size);
    assert_eq!(session.block_axis(root).height(), size.height);
    assert!(!session.context.pagination_on_transposed_axes);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        TransposedDocument::with_transposed(&mut session, root, axes, 800.0, |_, _| {
            std::panic::panic_any(719_u64)
        });
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<u64>(), Some(&719));
    assert_eq!(session.style_view(root).size(), size);
    assert_eq!(session.block_axis(root).height(), size.height);
    assert!(!session.context.pagination_on_transposed_axes);
    session.assert_style_queries_allowed();
    assert_eq!(session.style_view(root).size(), size);
}

#[test]
fn pagination_identity_change_is_rejected() {
    for (panic_body, mutation) in [
        (false, 0),
        (true, 0),
        (false, 1),
        (true, 1),
        (false, 2),
        (true, 2),
    ] {
        let mut doc = Document::new();
        let root = doc.append_element(Some(0), "html", None::<&str>);
        doc.append_element(Some(root), "div", None::<&str>);
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        let mut session = LayoutSession::bridged(&mut doc, &cascade);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TransposedDocument::with_transposed(
                &mut session,
                root,
                FlowAxes {
                    right_to_left: false,
                },
                800.0,
                |s, _| {
                    // Even replacing an empty generated range advances its identity.
                    match mutation {
                        0 => s.nodes.set_anonymous_cells(Vec::new()),
                        1 => {
                            s.append_element(Some(root), "div", None::<&str>);
                        }
                        _ => {
                            s.nodes.pop();
                            s.document.invalidate_layout_cache();
                        }
                    }
                    if panic_body {
                        std::panic::panic_any(719_u64);
                    }
                },
            );
        }))
        .unwrap_err();
        if panic_body {
            assert_eq!(panic.downcast_ref::<u64>(), Some(&719));
        }
        assert!(!session.context.pagination_on_transposed_axes);
        session.assert_style_queries_allowed();
    }
}

#[test]
fn pagination_entry_rejects_active_edge_scope() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "html", None::<&str>);
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_resolved_edges(root, 100.0, false, |s| {
            TransposedDocument::with_transposed(
                s,
                root,
                FlowAxes {
                    right_to_left: true,
                },
                800.0,
                |_, _| (),
            );
        });
    }));
    assert!(panic.is_err());
    assert!(session.context.resolved_edge_bases.is_empty());
    assert!(!session.context.pagination_on_transposed_axes);
}

/// The facts the paginator reads along its flow: height, its bounds, the
/// margins at the flow's start and end, and the aspect ratio.
type Facts = (
    Dimension,
    LengthPercentageAuto,
    LengthPercentageAuto,
    LengthPercentageAuto,
    LengthPercentageAuto,
    Option<f32>,
);

/// The facts the paginator reads for `key` now.
fn paginated_facts(session: &LayoutSession<'_>, key: usize) -> Facts {
    let axis = session.block_axis(key);
    (
        axis.height(),
        axis.min_height(),
        axis.max_height(),
        axis.margin_top(),
        axis.margin_bottom(),
        axis.aspect_ratio(),
    )
}

/// The facts of the physical view of `key`.
fn physical_facts(session: &LayoutSession<'_>, key: usize) -> Facts {
    let view = session.style_view(key);
    let (size, min, max, margin) = (view.size(), view.min_size(), view.max_size(), view.margin());
    (
        size.height,
        min.height,
        max.height,
        margin.top,
        margin.bottom,
        view.aspect_ratio(),
    )
}

/// The facts of the physical view of `key` with the physical sizes and sides
/// exchanged onto transposed axes.
fn swapped_facts(axes: FlowAxes, session: &LayoutSession<'_>, key: usize) -> Facts {
    let view = session.style_view(key);
    let margin = axes.rect(view.margin().into_edges());
    (
        view.size().width,
        view.min_size().width,
        view.max_size().width,
        margin.top,
        margin.bottom,
        view.aspect_ratio().map(f32::recip),
    )
}

#[test]
fn block_axis_facts_are_the_physical_style_on_transposed_axes() {
    // Boxes of every kind a vertical root can hold: physical and logical
    // sizes and margins, insets, a box on horizontal axes inside the vertical
    // flow, sideways text and a table.
    let children = [
        (
            "div",
            "width:10px;height:20px;min-width:3px;min-height:5px;max-width:40%;max-height:50px;margin:1px 2px 3px 4px",
        ),
        (
            "div",
            "position:absolute;top:1px;right:2px;bottom:auto;left:4px;overflow-x:hidden;overflow-y:scroll;aspect-ratio:3/2;height:50%",
        ),
        (
            "div",
            "margin-top:auto;margin-bottom:30%;width:auto;min-height:10%;max-height:none",
        ),
        (
            "div",
            "inline-size:30px;block-size:40px;min-block-size:7px;max-inline-size:90%;margin-block-start:5px;margin-inline-end:6px",
        ),
        (
            "div",
            "position:absolute;inset-block-start:3px;inset-inline-end:9px;aspect-ratio:2",
        ),
        (
            "div",
            "writing-mode:horizontal-tb;width:100px;height:60px;margin:2px 3px 4px 5px;overflow:hidden",
        ),
        (
            "div",
            "writing-mode:sideways-rl;width:40px;height:80px;padding:3px",
        ),
        ("table", "display:table;width:70px;margin:3px"),
    ];
    for root_css in ["writing-mode:vertical-rl", "writing-mode:vertical-lr"] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Some(format!("display:block;{root_css}").as_str()),
        );
        let body = doc.append_element(Some(html), "body", Some("display:block"));
        for (tag, css) in children {
            let child = doc.append_element(
                Some(body),
                tag,
                Some(format!("display:block;{css}").as_str()),
            );
            doc.append_text(child, "text");
        }
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        let mut session = LayoutSession::bridged(&mut doc, &cascade);
        let axes = FlowAxes::of_root(&session, &cascade).expect("a vertical root");
        let keys: Vec<usize> = session.nodes.layout_keys().collect();
        let physical: Vec<Facts> = keys
            .iter()
            .map(|&key| physical_facts(&session, key))
            .collect();
        for (&key, physical) in keys.iter().zip(&physical) {
            // Outside a transposed pagination the facts are the physical ones.
            assert_eq!(paginated_facts(&session, key), *physical, "key {key}");
        }
        // Native style queries are closed while a pagination is transposed, so
        // what the paginator should read is worked out beforehand.
        let swapped: Vec<Facts> = keys
            .iter()
            .map(|&key| swapped_facts(axes, &session, key))
            .collect();
        TransposedDocument::with_transposed(&mut session, body, axes, 800.0, |s, _| {
            let mut differs = false;
            for ((&key, physical), expected) in keys.iter().zip(&physical).zip(&swapped) {
                assert_eq!(paginated_facts(s, key), *expected, "key {key}");
                differs |= expected != physical;
            }
            assert!(differs, "the exchange of axes is observable");
        });
        for (&key, physical) in keys.iter().zip(&physical) {
            assert_eq!(paginated_facts(&session, key), *physical);
        }
    }
}
