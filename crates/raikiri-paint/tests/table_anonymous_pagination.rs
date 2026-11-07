//! Table parts outside any table, wrapped in an anonymous table, paginated
//! against the same parts inside a table that is written out.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_dom::{Document, PageContentInsets, PageMargins, PaintEvent, layout_pages};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;

const BORDER_MODELS: [&str; 2] = [
    "border-collapse:collapse",
    "border-collapse:separate;border-spacing:3px",
];

fn element(doc: &mut Document, parent: usize, style: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(style))
}

fn page() -> PageBox {
    let mut page = PageBox::new();
    page.width = 120.0;
    page.height = 100.0;
    page
}

/// Where the rows of the table sit among the boxes around them.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// The rows are children of the parent, with nothing between.
    Rows,
    /// All rows in one row group.
    Group,
    /// Two rows to a row group.
    Groups,
    /// The rows are the content of a cell of a table.
    InCell,
}

/// A document of `spacer` px of block, then `rows` rows of two cells with
/// borders and backgrounds, laid out as the rows of the table that
/// `explicit` says, or as rows outside any table. Returns the document and
/// the box around the rows: the table, or a block.
fn document(
    explicit: bool,
    border_model: &str,
    spacer: u32,
    rows: usize,
    shape: Shape,
) -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    element(&mut doc, body, &format!("display:block;height:{spacer}px"));
    let host = if matches!(shape, Shape::InCell) {
        let outer = element(&mut doc, body, &format!("display:table;{border_model}"));
        let outer_row = element(&mut doc, outer, "display:table-row");
        element(&mut doc, outer_row, "display:table-cell")
    } else {
        body
    };
    let display = if explicit { "table" } else { "block" };
    let parent = element(&mut doc, host, &format!("display:{display};{border_model}"));
    let mut container = parent;
    for index in 0..rows {
        let grouped = match shape {
            Shape::Group => index == 0,
            Shape::Groups => index % 2 == 0,
            Shape::Rows | Shape::InCell => false,
        };
        if grouped {
            container = element(
                &mut doc,
                parent,
                "display:table-row-group;background:rgb(200,230,200)",
            );
        }
        let row = element(
            &mut doc,
            container,
            "display:table-row;background:rgb(230,230,200)",
        );
        for column in 0..2 {
            let color = if (index + column) % 2 == 0 {
                "red"
            } else {
                "blue"
            };
            element(
                &mut doc,
                row,
                &format!(
                    "display:table-cell;width:20px;height:10px;border:2px solid {color};background:rgb(200,200,230)"
                ),
            );
        }
    }
    (doc, parent)
}

/// The paint order of every page of `doc`, with the boxes named by where they
/// are rather than by their nodes. The box `around` the rows is left out.
fn pages(doc: &mut Document, around: usize) -> Vec<Vec<String>> {
    doc.mark_in_document_flags();
    let rules = build_rule_tree(doc);
    let computed = cascade(doc, &rules).unwrap();
    let slices = layout_pages(doc, &computed, page()).unwrap();
    let margins: Vec<_> = slices
        .iter()
        .map(|_| (page(), PageMargins::default(), PageContentInsets::default()))
        .collect();
    doc.project_pages(&computed, page(), &slices, &margins)
        .unwrap();
    (0..slices.len() as u32)
        .map(|index| {
            doc.page_paint_order(&computed, index, None)
                .iter()
                .filter(|event| {
                    !matches!(event, PaintEvent::Box(fragment) if fragment.node().0 == around as u64)
                })
                .map(|event| match event {
                    PaintEvent::Box(fragment) => {
                        let rect = fragment.paint_rect();
                        let border = doc
                            .get_node(fragment.node().0 as usize)
                            .and_then(|node| node.collapsed_border())
                            .map(|border| {
                                [border.top, border.right, border.bottom, border.left]
                                    .map(|side| side.width().0)
                            });
                        format!(
                            "box ({}, {}, {}, {}) {border:?}",
                            rect.x, rect.y, rect.width, rect.height
                        )
                    }
                    PaintEvent::PushClip(clip, kind) => format!(
                        "clip ({}, {}, {}, {}) {kind:?}",
                        clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height
                    ),
                    PaintEvent::PopClip => "pop".to_string(),
                    _ => "other".to_string(),
                })
                .collect()
        })
        .collect()
}

#[test]
fn rows_outside_a_table_paginate_and_paint_like_the_rows_of_a_table() {
    for shape in [Shape::Rows, Shape::Group, Shape::Groups, Shape::InCell] {
        for border_model in BORDER_MODELS {
            // Rows that fit, rows that cross the first page edge, and rows
            // that fill several pages.
            for (spacer, rows) in [(10, 3), (70, 6), (85, 4), (0, 12)] {
                let (mut loose, loose_around) = document(false, border_model, spacer, rows, shape);
                let (mut explicit, explicit_around) =
                    document(true, border_model, spacer, rows, shape);
                let loose_pages = pages(&mut loose, loose_around);
                let explicit_pages = pages(&mut explicit, explicit_around);
                assert_eq!(
                    loose_pages, explicit_pages,
                    "{shape:?} {border_model} spacer={spacer} rows={rows}"
                );
            }
        }
    }
}

#[test]
fn a_row_outside_a_table_is_never_cut_by_a_page_edge() {
    for border_model in BORDER_MODELS {
        let (mut doc, around) = document(false, border_model, 85, 4, Shape::Rows);
        let order = pages(&mut doc, around);
        assert_eq!(order.len(), 2, "{border_model}");
        // The first row would end at 105: it starts the second page, and no
        // part of it stays on the first.
        let on_first = order[0]
            .iter()
            .filter(|event| event.starts_with("box"))
            .count();
        let on_second = order[1]
            .iter()
            .filter(|event| event.starts_with("box"))
            .count();
        assert!(on_first > 0 && on_second > 0, "{border_model}");
        assert!(
            order[0].iter().all(|event| !event.contains(", 95,")),
            "{border_model}: {:?}",
            order[0]
        );
    }
}

#[test]
fn collapsed_borders_of_rows_outside_a_table_paint_like_those_of_a_table() {
    // The borders of the cells meet and the outer ones are not left out or
    // painted twice.
    let render = |explicit: bool| {
        let (mut doc, _) = document(explicit, BORDER_MODELS[0], 0, 2, Shape::Rows);
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let computed = cascade(&doc, &rules).unwrap();
        raikiri_dom::layout_single_page(&mut doc, &computed, page()).unwrap();
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page(&mut scene, &doc, &computed, page()).unwrap();
        anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            120,
            100,
        )
    };
    let (loose, explicit) = (render(false), render(true));
    assert!(loose.chunks_exact(4).any(|pixel| pixel == [255, 0, 0, 255]));
    let differences = loose
        .chunks_exact(4)
        .zip(explicit.chunks_exact(4))
        .filter(|(loose, explicit)| loose != explicit)
        .count();
    assert_eq!(differences, 0);
}
