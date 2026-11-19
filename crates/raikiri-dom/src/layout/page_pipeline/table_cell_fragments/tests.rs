use super::*;

struct Fixture {
    doc: Document,
    body: usize,
    table: usize,
    group: usize,
    row: usize,
    cell: usize,
}

impl Fixture {
    fn new() -> Self {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Some("display:block;margin:0;font:20px/30px Ahem"),
        );
        let table = doc.append_element(Some(body), "div", Some("display:table;border-spacing:0"));
        let group = doc.append_element(Some(table), "div", Some("display:table-row-group"));
        let row = doc.append_element(Some(group), "div", Some("display:table-row"));
        let cell = doc.append_element(
            Some(row),
            "div",
            Some("display:table-cell;height:45px;vertical-align:middle"),
        );
        doc.append_text(cell, "T");
        Self {
            doc,
            body,
            table,
            group,
            row,
            cell,
        }
    }

    fn qualified(self) -> bool {
        self.qualified_on_axes(false)
    }

    /// The cells `split` moves the alignment room of, and by how much, for
    /// `node` placed at `top` on a page from 100 to 200.
    fn split_at(mut self, node: usize, top: f32, transposed: bool) -> Vec<(usize, f32)> {
        self.doc.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&self.doc, &raikiri_style::build_rule_tree(&self.doc)).unwrap();
        super::super::layout_single_page(
            crate::layout::test_support::with_ahem(&mut self.doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let mut session = crate::layout::context::LayoutSession::resumed(&mut self.doc, &cascade);
        session.context.pagination_on_transposed_axes = transposed;
        split(&session, &cascade, node, top, 100.0, 200.0)
            .into_iter()
            .map(|fragment| (fragment.cell, fragment.shift))
            .collect()
    }

    fn qualified_on_axes(mut self, transposed: bool) -> bool {
        self.doc.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&self.doc, &raikiri_style::build_rule_tree(&self.doc)).unwrap();
        super::super::layout_single_page(
            crate::layout::test_support::with_ahem(&mut self.doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let mut session = crate::layout::context::LayoutSession::resumed(&mut self.doc, &cascade);
        session.context.pagination_on_transposed_axes = transposed;
        fitting(&session, &cascade, self.row, 160.0, 100.0, 200.0)
    }
}

#[test]
fn a_real_single_line_cell_qualifies() {
    assert!(Fixture::new().qualified());
}

#[test]
fn other_cells_rows_groups_and_captions_keep_the_existing_path() {
    for level in 0..4 {
        let mut fixture = Fixture::new();
        let (parent, display) = match level {
            0 => (fixture.row, "table-cell"),
            1 => (fixture.group, "table-row"),
            2 => (fixture.table, "table-row-group"),
            _ => (fixture.table, "table-caption"),
        };
        fixture.doc.append_element(
            Some(parent),
            "div",
            Some(format!("display:{display};height:10px")),
        );
        assert!(!fixture.qualified(), "{display}");
    }
    for group in ["table-header-group", "table-footer-group"] {
        let mut fixture = Fixture::new();
        fixture
            .doc
            .set_element_attribute(fixture.group, "style", format!("display:{group}"))
            .unwrap();
        assert!(!fixture.qualified(), "{group}");
    }
    for span in ["rowspan", "colspan"] {
        let mut fixture = Fixture::new();
        fixture
            .doc
            .set_element_attribute(fixture.cell, span, "2")
            .unwrap();
        assert!(!fixture.qualified(), "{span}");
    }
}

#[test]
fn nested_inline_and_block_contents_are_not_plain_cell_text() {
    for display in ["inline", "block", "contents", "inline-block"] {
        let mut fixture = Fixture::new();
        let child = fixture.doc.append_element(
            Some(fixture.cell),
            "span",
            Some(format!("display:{display}")),
        );
        fixture.doc.append_text(child, "U");
        assert!(!fixture.qualified(), "{display}");
    }
}

#[test]
fn multiline_and_baseline_aligned_cells_keep_the_existing_path() {
    let mut multiline = Fixture::new();
    multiline
        .doc
        .set_element_attribute(
            multiline.cell,
            "style",
            "display:table-cell;height:75px;width:20px;vertical-align:middle",
        )
        .unwrap();
    multiline.doc.append_text(multiline.cell, " T");
    assert!(!multiline.qualified());

    let mut baseline = Fixture::new();
    baseline
        .doc
        .set_element_attribute(
            baseline.cell,
            "style",
            "display:table-cell;height:45px;vertical-align:baseline",
        )
        .unwrap();
    assert!(!baseline.qualified());
}

#[test]
fn avoidance_forced_breaks_positioned_offsets_and_clipped_cells_do_not_qualify() {
    for level in 0..5 {
        for css in [
            "break-inside:avoid",
            "break-before:page",
            "break-after:page",
            "position:relative;top:10px",
            "position:absolute",
            "position:fixed",
        ] {
            let mut fixture = Fixture::new();
            let (node, display) = match level {
                0 => (fixture.cell, "table-cell;height:45px;vertical-align:middle"),
                1 => (fixture.row, "table-row"),
                2 => (fixture.group, "table-row-group"),
                3 => (fixture.table, "table"),
                _ => (fixture.body, "block"),
            };
            fixture
                .doc
                .set_element_attribute(node, "style", format!("display:{display};{css}"))
                .unwrap();
            assert!(!fixture.qualified(), "{display} {css}");
        }
    }
    for css in [
        "overflow:hidden",
        "writing-mode:vertical-rl",
        "column-count:2",
    ] {
        let mut fixture = Fixture::new();
        fixture
            .doc
            .set_element_attribute(
                fixture.cell,
                "style",
                format!("display:table-cell;height:45px;vertical-align:middle;{css}"),
            )
            .unwrap();
        assert!(!fixture.qualified(), "{css}");
    }
}

#[test]
fn generated_cell_content_keeps_the_existing_path() {
    for (pseudo, extra) in [
        ("before", ""),
        ("after", ""),
        ("before", "display:block;break-before:page"),
    ] {
        let mut fixture = Fixture::new();
        fixture
            .doc
            .set_element_attribute(fixture.cell, "id", "cell")
            .unwrap();
        let style = fixture
            .doc
            .append_element(Some(fixture.body), "style", Some("display:none"));
        fixture.doc.append_text(
            style,
            format!("#cell::{pseudo} {{ content: 'U'; {extra} }}"),
        );
        assert!(!fixture.qualified(), "::{pseudo}");
    }
}

#[test]
fn transposed_pagination_keeps_the_existing_path() {
    assert!(!Fixture::new().qualified_on_axes(true));
}

#[test]
fn a_split_cell_moves_the_room_its_alignment_put_above_its_line() {
    let fixture = Fixture::new();
    let (row, cell) = (fixture.row, fixture.cell);
    // A 30px line in the middle of 45px leaves 7.5px above it.
    assert_eq!(fixture.split_at(row, 160.0, false), [(cell, 7.5)]);
}

#[test]
fn cells_that_end_on_their_page_or_have_no_room_above_stay() {
    let fixture = Fixture::new();
    let row = fixture.row;
    assert!(fixture.split_at(row, 120.0, false).is_empty());

    let mut top = Fixture::new();
    top.doc
        .set_element_attribute(
            top.cell,
            "style",
            "display:table-cell;height:45px;vertical-align:top",
        )
        .unwrap();
    let row = top.row;
    assert!(top.split_at(row, 160.0, false).is_empty());
}

#[test]
fn vertical_cells_other_boxes_and_transposed_pagination_are_not_split() {
    let mut vertical = Fixture::new();
    vertical
        .doc
        .set_element_attribute(
            vertical.cell,
            "style",
            "display:table-cell;height:45px;vertical-align:middle;writing-mode:vertical-rl",
        )
        .unwrap();
    let row = vertical.row;
    assert!(vertical.split_at(row, 160.0, false).is_empty());

    let fixture = Fixture::new();
    let group = fixture.group;
    assert!(fixture.split_at(group, 160.0, false).is_empty());

    let fixture = Fixture::new();
    let row = fixture.row;
    assert!(fixture.split_at(row, 160.0, true).is_empty());
}

/// Whether a forced break falls inside a cell when a box styled `before`, with
/// `text` in it, sits ahead of a block that forces a break before itself.
fn breaks_after(before: &str, text: &str) -> bool {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = doc.append_element(Some(body), "div", Some("display:table"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell"));
    let first = doc.append_element(Some(cell), "div", Some(before));
    doc.append_text(first, text);
    let block = doc.append_element(Some(cell), "div", Some("display:block;break-before:page"));
    doc.append_text(block, "B");
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    super::super::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
    breaks_inside(&session, &cascade, cell)
}

#[test]
fn floats_positioned_and_contents_boxes_are_not_content_a_break_falls_after() {
    assert!(breaks_after("display:block", "Q"));
    assert!(!breaks_after("display:block;position:absolute", "Q"));
    assert!(!breaks_after("display:block;float:left;width:10px", "Q"));
    assert!(!breaks_after("display:contents", " "));
    assert!(breaks_after("display:contents", "Q"));
}

/// Whether a float that avoids a break inside it, after a block of 40px at
/// the top of a cell 100px wide placed at 30px in the flow, crosses a page
/// end at 100px. The float sits in a `span` when `in_span`, and `after`
/// holds the style and text of the boxes after it.
fn avoided_float_crosses(in_span: bool, after: &[(&str, &str)]) -> bool {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:10px/20px Ahem"),
    );
    let table = doc.append_element(Some(body), "div", Some("display:table"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell;width:100px"));
    doc.append_element(Some(cell), "div", Some("display:block;height:40px"));
    let parent = if in_span {
        let paragraph = doc.append_element(Some(cell), "div", Some("display:block"));
        doc.append_text(paragraph, "T");
        doc.append_element(Some(paragraph), "span", Some("display:inline"))
    } else {
        cell
    };
    doc.append_element(
        Some(parent),
        "div",
        Some("display:block;float:left;width:20px;height:40px;break-inside:avoid"),
    );
    for &(style, text) in after {
        let box_after = doc.append_element(Some(cell), "div", Some(style));
        doc.append_text(box_after, text);
    }
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    super::super::layout_single_page(
        crate::layout::test_support::with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let session = crate::layout::context::LayoutSession::resumed(&mut doc, &cascade);
    let offset = 30.0 - current_abs_y(&session, cell);
    float_crosses(&session, &cascade, cell, cell, offset, 100.0)
}

#[test]
fn an_avoided_float_moves_down_in_its_cell_alone() {
    assert!(!avoided_float_crosses(false, &[]));
    // A box that stays above the page end beside it.
    assert!(!avoided_float_crosses(
        false,
        &[("display:block;height:20px", "Y")]
    ));
    // A later float that crosses the page end and moves with it.
    assert!(!avoided_float_crosses(
        false,
        &[(
            "display:block;float:left;width:20px;height:40px;break-inside:avoid",
            "G"
        )]
    ));
    // Lines that the float puts below itself come back up when it moves.
    assert!(!avoided_float_crosses(
        false,
        &[("display:block;width:10px;height:20px", "YY YY YY")]
    ));
}

#[test]
fn an_avoided_float_stays_for_what_lays_out_beside_or_below_it() {
    // The lines of its own paragraph.
    assert!(avoided_float_crosses(true, &[]));
    // A later float that stays on the page.
    assert!(avoided_float_crosses(
        false,
        &[("display:block;float:left;width:20px;height:20px", "G")]
    ));
    // A box that clears.
    assert!(avoided_float_crosses(
        false,
        &[("display:block;clear:left", "Y")]
    ));
    // Lines beside it that run past the page end.
    assert!(avoided_float_crosses(
        false,
        &[("display:block", "AAA BBB CCC DDD EEE")]
    ));
    // An independent formatting context beside it that runs past the page end.
    assert!(avoided_float_crosses(
        false,
        &[("display:flow-root;height:40px", "Y")]
    ));
}
