use super::PARENT_COMPARISONS;
use crate::Document;
use raikiri_style::{Origin, RuleTree};

fn document(columns: usize, extra: &str, wrapper_css: &str, prefix: bool) -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;width:{}px;columns:{columns};column-gap:0;{extra}",
            columns * 10
        )),
    );
    if prefix {
        doc.append_element(Some(container), "div", Some("display:block;height:40px"));
    }
    let wrapper = doc.append_element(
        Some(container),
        "div",
        Some(format!("display:block;{wrapper_css}")),
    );
    let inner = doc.append_element(Some(wrapper), "div", Some("display:block"));
    doc.append_element(
        Some(inner),
        "div",
        Some(format!(
            "display:block;width:10px;height:{}px",
            columns * 10
        )),
    );
    doc.mark_in_document_flags();
    doc
}

fn layout(doc: &mut Document, columns: usize) {
    let mut rules = RuleTree::empty();
    rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
    let cascade = raikiri_style::cascade(doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = columns as f32 * 10.0 + 20.0;
    page.height = 600.0;
    crate::layout_single_page(doc, &cascade, page).unwrap();
}

#[test]
fn wrapped_parent_column_work_scales_with_emitted_pieces() {
    for columns in [128, 256, 512, 1024, 4096] {
        let mut doc = document(columns, "", "", false);
        PARENT_COMPARISONS.with(|work| work.set([0; 2]));
        layout(&mut doc, columns);
        let counts = PARENT_COMPARISONS.with(|work| work.get());
        println!("columns={columns} parent_comparisons={counts:?}");
        assert_eq!(doc.node_count(), 7);
        assert!(counts[0] >= columns);
        assert!(counts[1] >= columns);
        assert!(counts[0] <= 2 * columns, "wrapper {columns}: {counts:?}");
        assert!(counts[1] <= 2 * columns, "block {columns}: {counts:?}");
    }
}

#[test]
fn wrapper_producer_keeps_contiguous_columns_at_rounding_and_inset_boundaries() {
    use super::super::{Piece, wrapper_pieces};
    for start in [0, 3, 128] {
        for span in [0, 1, 129] {
            for (leading, trailing) in [(0.0, 0.0), (0.25, 0.75), (1.0, 1.0)] {
                let first = Piece {
                    column: start,
                    top: 1.0,
                    height: 0.0,
                };
                let last = Piece {
                    column: start + span,
                    top: 1.0,
                    height: 0.0,
                };
                let pieces = wrapper_pieces(&first, &last, leading, trailing, 10.0).unwrap();
                let initial = pieces.first().unwrap().column;
                for (index, piece) in pieces.iter().enumerate() {
                    assert_eq!(piece.column, initial + index);
                    assert_eq!(piece.column.checked_sub(initial), Some(index));
                    assert!(piece.height >= 0.0);
                }
            }
        }
    }
    // The end exactly on a column boundary belongs to the preceding column.
    let first = Piece {
        column: 5,
        top: 0.0,
        height: 10.0,
    };
    let pieces = wrapper_pieces(&first, &first, 0.0, 0.0, 10.0).unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].column, 5);
}

#[test]
fn wrapped_parent_fragments_match_columns_with_insets_overflow_and_nonzero_start() {
    for (extra, decoration, prefix, exercised) in [
        ("", "", false, true),
        ("", "", true, true),
        ("height:9px;column-fill:auto", "", false, true),
        (
            "",
            "padding-top:2px;padding-bottom:3px;border-top:1px solid;border-bottom:1px solid;box-decoration-break:slice",
            false,
            true,
        ),
        ("direction:rtl", "", false, false),
        (
            "",
            "padding-top:2px;padding-bottom:3px;box-decoration-break:clone",
            false,
            false,
        ),
    ] {
        let mut doc = document(128, extra, decoration, prefix);
        PARENT_COMPARISONS.with(|work| work.set([0; 2]));
        layout(&mut doc, 128);
        let counts = PARENT_COMPARISONS.with(|work| work.get());
        let fragments = doc.layout_fragments();
        assert_eq!(counts.iter().all(|&count| count > 0), exercised);
        if !exercised {
            continue;
        }
        let wrapper = if prefix { 5 } else { 4 };
        let inner = wrapper + 1;
        let leaf = inner + 1;
        for node in [inner, leaf] {
            let pieces = fragments
                .iter()
                .filter(|f| f.node_id == node)
                .collect::<Vec<_>>();
            assert!(pieces.len() > 1);
            if prefix {
                assert!(pieces[0].fragmentainer > 0);
            }
            for (index, piece) in pieces.iter().enumerate() {
                let parent = &fragments[piece.parent.unwrap()];
                assert_eq!(parent.node_id, node - 1);
                assert_eq!(parent.fragmentainer, piece.fragmentainer);
                assert_eq!(piece.fragment_index, index);
                assert_eq!(piece.fragment_count, pieces.len());
                assert!(piece.fragmentainer_clip.is_some());
                assert!(piece.rect.height >= 0.0);
                assert_eq!(piece.line_start, None);
                assert_eq!(piece.line_end, None);
            }
        }
    }
}

#[test]
fn wrapped_columns_still_enforce_fragment_limits() {
    let mut doc = document(128, "", "", false);
    doc.lower_layout_fragment_limit(16);
    let mut rules = RuleTree::empty();
    rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 1300.0;
    page.height = 600.0;
    assert!(crate::layout_single_page(&mut doc, &cascade, page).is_err());
    assert!(doc.layout_fragments().len() <= 16);
}
