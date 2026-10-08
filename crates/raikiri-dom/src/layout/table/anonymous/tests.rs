use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

pub(super) fn record_owner_clone(source: &Node) {
    OWNER_CLONE_ENTRIES.with(|entries| {
        let attributes = match &source.data {
            crate::node::NodeData::Element(element) => element.attributes.len(),
            _ => 0,
        };
        entries.set(entries.get() + source.children.len() + attributes);
    });
}

#[test]
fn deep_contents_wrappers_keep_source_order_on_a_small_stack() {
    let mut doc = Document::new();
    let owner = doc.append_element(Some(0), "div", Style::default(), Some("display:table-row"));
    let first = doc.append_text(owner, "A");
    let mut parent = owner;
    for _ in 0..2048 {
        parent = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:contents"),
        );
    }
    let middle = doc.append_text(parent, "B");
    let last = doc.append_text(owner, "C");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let actual = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || children(&doc, &computed, owner, &mut TableObjects::default()))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            Content::Node(first),
            Content::Node(middle),
            Content::Node(last)
        ]
    );
}

#[test]
fn owner_generated_cells_keep_source_children_and_shared_projection_limits() {
    use crate::layout::ifc::error::IfcError;
    use crate::layout::ifc::projection::{GeneratedCounters, project_anonymous_cell_builder};
    use shodo::limits::{LimitKind, Limits};

    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(
        sheet,
        "#owner::before{content:'X'}#owner::after{content:'Y'}",
    );
    let table = doc.append_element(Some(0), "div", Style::default(), Some("display:table"));
    let owner = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    doc.set_element_attributes(owner, vec![("id".into(), "owner".into())]);
    let proper = doc.append_element(
        Some(owner),
        "div",
        Style::default(),
        Some("display:table-cell"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(doc.nodes[owner].children, vec![proper]);
    assert_eq!(doc.table_objects.cells.len(), 2);
    assert_eq!(
        doc.table_objects.cells[0].content,
        vec![Content::Before(owner)]
    );
    assert_eq!(
        doc.table_objects.cells[1].content,
        vec![Content::After(owner)]
    );
    let fonts = crate::layout::ifc::test_support::ahem_fonts();
    for cell in &doc.table_objects.cells {
        assert!(cell.node.children.is_empty());
        let border = cell.node.computed_border.unwrap();
        assert_eq!(
            border.top.style(),
            raikiri_style::property::BorderStyle::None
        );
        assert_eq!(border.top.width().0, 0.0);
        for (limits, expected) in [
            (
                Limits {
                    max_items: Some(0),
                    ..Limits::default()
                },
                LimitKind::Items,
            ),
            (
                Limits {
                    max_text_bytes: Some(0),
                    ..Limits::default()
                },
                LimitKind::TextBytes,
            ),
        ] {
            let result = project_anonymous_cell_builder(
                &doc,
                &computed,
                owner,
                &cell.content,
                &fonts,
                &limits,
                &GeneratedCounters::default(),
            );
            assert!(matches!(result, Err(IfcError::Limit(error)) if error.kind == expected));
        }
        assert!(
            project_anonymous_cell_builder(
                &doc,
                &computed,
                owner,
                &cell.content,
                &fonts,
                &Limits::default(),
                &GeneratedCounters::default(),
            )
            .is_ok()
        );
    }
    assert_eq!(doc.nodes[owner].children, vec![proper]);
}

#[test]
fn source_owner_enumeration_is_linear_across_many_anonymous_rows() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let table = doc.append_element(Some(body), "div", Style::default(), Some("display:table"));
    let mut owners = Vec::new();
    for _ in 0..200 {
        let row = doc.append_element(
            Some(table),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
        doc.append_text(row, "A");
        doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell"),
        );
        owners.push(row);
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (node, cv) in doc.nodes.iter_mut().zip(&computed.computed) {
        node.display = cv.display;
    }
    prepare(&mut doc, &computed).unwrap();
    OWNER_CELL_VISITS.with(|visits| visits.set(0));
    for owner in owners {
        let cells: Vec<_> = doc.anonymous_table_cells(owner).collect();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].1.children, vec![doc.nodes[owner].children[0]]);
        assert_eq!(
            doc.anonymous_table_paint_children(owner).unwrap(),
            doc.nodes[owner].children
        );
    }
    assert_eq!(doc.anonymous_table_cells(body).count(), 0);
    assert_eq!(doc.anonymous_table_paint_children(body), None);
    let visited = OWNER_CELL_VISITS.with(|visits| visits.get());
    assert!(
        visited <= doc.node_count() * 3,
        "owner lookup visited {visited} cells, exceeding the document-wide work bound"
    );
}

#[test]
fn generated_contents_classification_preserves_css22_visibility_and_rebuilds() {
    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "span::before{content:'X'}");
    let table = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some("display:table;border-collapse:separate;empty-cells:hide"),
    );
    let row = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    let span = doc.append_element(
        Some(row),
        "span",
        Style::default(),
        Some("display:contents"),
    );
    for (owner_visibility, pseudo_visibility, expected) in [
        ("visible", "visible", false),
        ("hidden", "visible", true),
        ("visible", "hidden", true),
        ("visible", "visible", false),
    ] {
        doc.set_element_inline_style(
            row,
            Some(format!("display:table-row;visibility:{owner_visibility}").into()),
        );
        doc.set_element_inline_style(
            span,
            Some(format!("display:contents;visibility:{pseudo_visibility}").into()),
        );
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        assert_eq!(doc.table_objects.cells.len(), 1);
        let cell = &doc.table_objects.cells[0];
        assert!(cell.node.children.is_empty());
        assert_eq!(cell.content, vec![Content::Before(span)]);
        assert_eq!(cell.node.hides_empty_table_cell, expected);
    }
}

#[test]
fn deep_generated_contents_stream_is_bounded_and_keeps_real_children() {
    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(sheet, "span::before{content:'X'}span::after{content:'Y'}");
    let owner = doc.append_element(Some(0), "div", Style::default(), Some("display:table-row"));
    let mut parent = owner;
    let mut wrappers = Vec::new();
    for _ in 0..2048 {
        parent = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        wrappers.push(parent);
    }
    let text = doc.append_text(parent, "A");
    let sources: Vec<_> = doc.nodes.iter().map(|node| node.children.clone()).collect();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (node, cv) in doc.nodes.iter_mut().zip(&computed.computed) {
        node.display = cv.display;
    }
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let actual = children(&doc, &computed, owner, &mut TableObjects::default());
            let expected: Vec<_> = wrappers
                .iter()
                .copied()
                .map(Content::Before)
                .chain(std::iter::once(Content::Node(text)))
                .chain(wrappers.iter().rev().copied().map(Content::After))
                .collect();
            assert_eq!(actual, expected);
            assert!(actual.len() <= doc.node_count() * 3);
            assert_eq!(
                doc.nodes
                    .iter()
                    .map(|node| node.children.clone())
                    .collect::<Vec<_>>(),
                sources
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn owner_clone_volume_is_linear_across_many_anonymous_rows() {
    for count in [200, 400] {
        let mut doc = Document::new();
        let table = doc.append_element(Some(0), "div", Style::default(), Some("display:table"));
        for index in 0..count {
            doc.set_element_attribute(table, format!("data-item-{index}"), "source")
                .unwrap();
            doc.append_element(Some(table), "div", Style::default(), Some("display:block"));
            doc.append_element(
                Some(table),
                "div",
                Style::default(),
                Some("display:table-row"),
            );
        }
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        OWNER_CLONE_ENTRIES.with(|entries| entries.set(0));
        crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
        assert_eq!(doc.anonymous_table_cells(table).count(), count);
        assert_eq!(doc.get_node(table).unwrap().children.len(), count * 2);
        assert_eq!(doc.element_attribute(table, "data-item-0"), Some("source"));
        let copied = OWNER_CLONE_ENTRIES.with(|entries| entries.get());
        assert!(
            copied <= (doc.node_count() + count) * 3,
            "owner cloning copied {copied} entries for {count} cells"
        );
    }
}

#[test]
fn column_bounds_are_checked_before_projection_and_recover_without_stale_cells() {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "div", Style::default(), Some("display:table"));
    let row = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    for _ in 0..65 {
        let cell = doc.append_element(
            Some(row),
            "div",
            Style::default(),
            Some("display:table-cell"),
        );
        doc.set_element_attribute(cell, "colspan", "1000").unwrap();
    }
    let last = doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:table-cell"),
    );
    doc.set_element_attribute(last, "colspan", "534").unwrap();
    let content = doc.append_element(Some(row), "div", Style::default(), Some("display:block"));
    doc.set_element_attribute(table, "data-source", "kept")
        .unwrap();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(
        super::super::build_table_grid(&doc, table).unwrap().n_cols,
        u16::MAX
    );
    assert_eq!(doc.anonymous_table_cells(row).count(), 1);

    let extra = doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:table-cell"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(crate::layout::apply_computed_to_style(&mut doc, &computed).is_err());
    assert!(doc.table_objects.cells.is_empty());
    assert!(doc.table_objects.rows.is_empty());
    assert_eq!(doc.element_attribute(table, "data-source"), Some("kept"));
    assert_eq!(doc.get_node(content).unwrap().parent, Some(row));
    doc.detach_from_parent(extra);
    doc.set_element_attribute(last, "colspan", "535").unwrap();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(crate::layout::apply_computed_to_style(&mut doc, &computed).is_err());
    assert!(doc.table_objects.cells.is_empty());
    doc.set_element_attribute(last, "colspan", "534").unwrap();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(
        super::super::build_table_grid(&doc, table).unwrap().n_cols,
        u16::MAX
    );
    assert_eq!(doc.anonymous_table_cells(row).count(), 1);
}

#[test]
fn row_bounds_are_checked_before_projection_at_the_existing_exact_boundary() {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "div", Style::default(), Some("display:table"));
    let group = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row-group"),
    );
    for index in 0..=u16::MAX {
        doc.append_element(
            Some(if index < 32768 { table } else { group }),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(
        doc.table_objects.rows[&table].len(),
        usize::from(u16::MAX) + 1
    );
    assert_eq!(
        super::super::build_table_grid(&doc, table)
            .unwrap()
            .rows
            .len(),
        65536
    );
    let extra = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(crate::layout::apply_computed_to_style(&mut doc, &computed).is_err());
    assert!(doc.table_objects.rows.is_empty());
    doc.detach_from_parent(extra);
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    assert_eq!(doc.table_objects.rows[&table].len(), 65536);
}

#[test]
fn prototypes_exclude_real_canvas_payloads_and_rebuild_with_source_changes() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let table = doc.append_element(
        Some(body),
        "canvas",
        Style::default(),
        Some("display:table;border-spacing:0"),
    );
    doc.set_element_attribute(table, "width", "32").unwrap();
    doc.set_element_attribute(table, "height", "16").unwrap();
    doc.set_element_attribute(table, "lang", "tr").unwrap();
    assert_eq!(doc.try_ensure_canvas_bitmap(table).unwrap(), Some((32, 16)));
    assert!(doc.canvas_fill_rect(table, 0, 0, 32, 16, [1, 2, 3, 255]));
    let block = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:10px"),
    );
    let row = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:10px"),
    );
    let children = doc.nodes[table].children.clone();
    for mode in ["horizontal-tb", "vertical-rl", "horizontal-tb"] {
        doc.set_element_inline_style(
            table,
            Some(format!("display:table;border-spacing:0;writing-mode:{mode}").into()),
        );
        doc.mark_in_document_flags();
        let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        assert!(anonymous_prototype(&doc, table).ifc.is_none());
        crate::layout::layout_single_page(&mut doc, &computed, raikiri_traits::PageBox::A4)
            .unwrap();
        assert_eq!(doc.anonymous_table_cells(table).count(), 2);
        assert!(doc.table_objects.prototypes_by_owner.is_empty());
        for (_, cell) in doc.anonymous_table_cells(table) {
            let crate::node::NodeData::Element(element) = &cell.data else {
                panic!("anonymous cell element");
            };
            assert!(element.attributes.is_empty());
            assert!(element.canvas_bitmap.is_none());
            assert!(cell.order_modified_children.is_empty());
            assert!(cell.grid_item_row_starts.is_empty());
            assert_eq!(cell.style.display, taffy::Display::Block);
            assert_eq!(
                cell.authored_writing_mode,
                doc.nodes[table].authored_writing_mode
            );
        }
        assert_eq!(doc.nodes[table].children, children);
        assert_eq!(doc.element_attribute(table, "lang"), Some("tr"));
        assert_eq!(
            doc.canvas_bitmap_ref(table).unwrap().rgba,
            [1, 2, 3, 255].repeat(512)
        );
        assert_eq!(doc.nodes[block].parent, Some(table));
        assert_eq!(doc.nodes[row].parent, Some(table));
    }
}

#[test]
fn deep_overlay_contents_metadata_is_linear_and_keeps_source_children() {
    let mut doc = Document::new();
    let sheet = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
    doc.append_text(
        sheet,
        "span::before{content:'X';position:absolute}span::after{content:'Y';float:left}",
    );
    let table = doc.append_element(Some(0), "div", Style::default(), Some("display:table"));
    let owner = doc.append_element(
        Some(table),
        "div",
        Style::default(),
        Some("display:table-row"),
    );
    let mut parent = owner;
    let mut wrappers = Vec::new();
    for _ in 0..2048 {
        parent = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:contents"),
        );
        wrappers.push(parent);
    }
    let text = doc.append_text(parent, "A");
    let sources: Vec<_> = doc.nodes.iter().map(|node| node.children.clone()).collect();
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            prepare(&mut doc, &computed).unwrap();
            assert_eq!(doc.anonymous_table_cells(owner).count(), 1);
            assert_eq!(
                doc.table_objects.cells[0].content,
                vec![Content::Node(text)]
            );
            assert_eq!(
                doc.table_objects.overlay_contents_by_owner[&owner],
                wrappers
            );
            assert_eq!(
                doc.table_objects.overlay_contents_owner.len(),
                wrappers.len()
            );
            let sequence = doc.anonymous_table_paint_sequence(owner).unwrap();
            assert_eq!(sequence.len(), wrappers.len() + 1);
            assert_eq!(sequence[0], text);
            assert_eq!(&sequence[1..], wrappers);
            assert!(sequence.len() <= doc.node_count() * 2);
            assert_eq!(
                doc.nodes
                    .iter()
                    .map(|node| node.children.clone())
                    .collect::<Vec<_>>(),
                sources
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
