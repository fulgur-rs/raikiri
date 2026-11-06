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

    fn qualified(mut self) -> bool {
        self.doc.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&self.doc, &raikiri_style::build_rule_tree(&self.doc)).unwrap();
        super::super::layout_single_page(
            crate::layout::test_support::with_ahem(&mut self.doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let session = crate::layout::context::LayoutSession::resumed(&mut self.doc, &cascade);
        fitting(&session, &cascade, self.row, 160.0, 100.0, 200.0).is_some()
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
