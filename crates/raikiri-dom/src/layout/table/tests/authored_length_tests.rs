use super::*;

#[test]
fn authored_length_checks_visit_each_cell_once() {
    let mut observations = Vec::new();
    for (rows, columns) in [(2u16, 2u16), (64, 8), (128, 32)] {
        let mut doc = Document::new();
        let mut cells = Vec::new();
        for row in 0..rows {
            for column in 0..columns {
                let id = doc.append_element(Some(0), "td", Style::default(), None::<&str>);
                cells.push(make_cell(id, row, column, 1, 1, Dimension::auto()));
            }
        }
        doc.mark_in_document_flags();
        let grid = super::super::TableGrid {
            n_cols: columns,
            rows: vec![],
            cells,
            col_widths: vec![],
        };
        AUTHORED_LENGTH_CELL_VISITS.with(|count| count.set(0));
        let widths = resolve_column_widths_in_session(
            &mut doc,
            &grid,
            column_probe_input(),
            Size::ZERO,
            0.0,
        );
        assert_eq!(widths, vec![0.0; usize::from(columns)]);
        let visits = AUTHORED_LENGTH_CELL_VISITS.with(|count| count.get());
        eprintln!("{rows}x{columns}: {visits} authored-length cell checks");
        observations.push((rows, columns, visits, grid.cells.len()));
    }
    assert!(
        observations
            .iter()
            .all(|&(_, _, visits, cells)| visits == cells),
        "{observations:?}"
    );
}

#[test]
fn later_definite_cell_keeps_spanning_excess_off_its_column() {
    let mut doc = Document::new();
    let auto = doc.append_element(Some(0), "td", Style::default(), None::<&str>);
    let definite = doc.append_element(
        Some(0),
        "td",
        Style {
            size: Size {
                width: Dimension::length(20.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let span = doc.append_element(Some(0), "td", Style::default(), None::<&str>);
    doc.append_element(
        Some(span),
        "div",
        Style {
            size: Size {
                width: Dimension::length(100.0),
                height: Dimension::length(10.0),
            },
            ..Style::default()
        },
        None::<&str>,
    );
    doc.mark_in_document_flags();
    let grid = super::super::TableGrid {
        n_cols: 2,
        rows: vec![],
        cells: vec![
            make_cell(auto, 0, 0, 1, 1, Dimension::auto()),
            make_cell(definite, 1, 0, 1, 1, Dimension::length(20.0)),
            make_cell(span, 2, 0, 2, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let widths =
        resolve_column_widths_in_session(&mut doc, &grid, column_probe_input(), Size::ZERO, 0.0);
    assert_eq!(widths, vec![20.0, 80.0]);
}
