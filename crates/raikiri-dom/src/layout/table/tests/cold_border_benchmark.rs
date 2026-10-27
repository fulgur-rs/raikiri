use super::super::BORDER_RESOLUTIONS;
use super::*;
use crate::layout::test_support::TABLE_CELL_LOOKUP_STEPS;

fn fixture(
    rows: usize,
    columns: usize,
    span: usize,
    row_span: bool,
) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let table = doc.append_element(Some(body), "table", Some("display:table;border-collapse:collapse;table-layout:fixed;width:600px;border:3px solid red"));
    for row in 0..rows {
        let tr = doc.append_element(Some(table), "tr", Some("display:table-row"));
        if row_span && row > 0 {
            continue;
        }
        for column in 0..columns {
            let color = if column % 2 == 0 { "blue" } else { "green" };
            let cell = doc.append_element(
                Some(tr),
                "td",
                Some(format!(
                    "display:table-cell;height:1px;border:2px solid {color}"
                )),
            );
            doc.set_element_attribute(cell, "colspan", span.to_string())
                .unwrap();
            if row_span {
                doc.set_element_attribute(cell, "rowspan", rows.to_string())
                    .unwrap();
            }
        }
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    (doc, computed, table)
}

#[test]
#[ignore = "isolated release cold-layout measurement"]
fn cold_table_border_benchmark() {
    for (rows, columns, span, row_span) in [
        (8, 1, 16, false),
        (8, 1, 1000, false),
        (32, 1, 16, false),
        (32, 1, 1000, false),
        (128, 1, 1000, false),
        (16, 1, 1000, true),
        (1000, 1, 1000, true),
        (8, 8, 1, false),
        (32, 16, 1, false),
        (64, 64, 1, false),
    ] {
        let mut elapsed = Vec::new();
        let mut fingerprint = None;
        let mut last_counts = (0, 0);
        for _ in 0..9 {
            let (mut doc, computed, table) = fixture(rows, columns, span, row_span);
            TABLE_CELL_LOOKUP_STEPS.set(0);
            BORDER_RESOLUTIONS.set(0);
            let start = std::time::Instant::now();
            crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();
            elapsed.push(start.elapsed().as_nanos());
            last_counts = (TABLE_CELL_LOOKUP_STEPS.get(), BORDER_RESOLUTIONS.get());
            let mut actual = Vec::new();
            for node in &doc.nodes {
                actual.extend([
                    node.layout.location.x.to_bits(),
                    node.layout.location.y.to_bits(),
                    node.layout.size.width.to_bits(),
                    node.layout.size.height.to_bits(),
                ]);
                if let Some(paint) = node.collapsed_border {
                    for border in [paint.top, paint.right, paint.bottom, paint.left] {
                        actual.push(border.width().px().to_bits());
                    }
                }
            }
            let hash = actual
                .into_iter()
                .fold(1469598103934665603u64, |hash, value| {
                    hash.wrapping_mul(1099511628211)
                        .wrapping_add(u64::from(value))
                });
            let value = (
                hash,
                doc.nodes[table].layout.size.width.to_bits(),
                doc.nodes[table].layout.size.height.to_bits(),
            );
            if let Some(expected) = fingerprint {
                assert_eq!(value, expected);
            } else {
                fingerprint = Some(value);
            }
        }
        elapsed.sort_unstable();
        eprintln!(
            "COLD rows={rows} cells_per_row={columns} colspan={span} rowspan={row_span} ns_median={} ns_min={} lookup={} resolution={} fingerprint={fingerprint:?}",
            elapsed[4], elapsed[0], last_counts.0, last_counts.1
        );
    }
}
