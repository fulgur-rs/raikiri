use super::*;

#[test]
fn placing_table_row_at_exact_page_offset_preserves_target() {
    // Crossing a power of two doubles the spacing of representable document coordinates.
    for (page_y, old_offset, new_offset) in [
        (65_106.15_f32, 429.85547, 467.0),
        (130_212.3_f32, 859.71094, 889.0),
    ] {
        let mut document = Document::new();
        let table = document.append_element(Some(0), "table", None::<&str>);
        let group = document.append_element(Some(table), "tbody", None::<&str>);
        let row = document.append_element(Some(group), "tr", None::<&str>);
        let cell = document.append_element(Some(row), "td", None::<&str>);
        document.nodes[group].layout.location.y = page_y;
        document.nodes[row].layout.location.y = old_offset;
        let target = page_y + new_offset;
        materialize_table_row_y(&mut document, row, target);
        assert_eq!(current_abs_y(&document, row), target);
        assert_eq!(current_abs_y(&document, cell), target);
    }
}
