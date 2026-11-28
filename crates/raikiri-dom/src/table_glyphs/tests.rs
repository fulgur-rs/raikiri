use super::*;

#[test]
fn measured_glyphs_match_projected_text_and_require_compatible_sample() {
    let mut document = crate::Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        Some("display:block;font:10px Ahem;text-decoration:underline"),
    );
    document.append_text(body, "123");
    let fonts = crate::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    )))
    .unwrap();
    document.set_font_collection(fonts);
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let page = raikiri_traits::PageBox::A4;
    crate::layout_single_page(&mut document, &cascade, page).unwrap();
    document
        .project_pages(
            &cascade,
            page,
            &[crate::PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[],
        )
        .unwrap();
    let store = store::Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        4096,
    );
    let paragraph = &document
        .get_node(body)
        .unwrap()
        .ifc
        .as_ref()
        .unwrap()
        .paragraph;
    capture(&store, body, paragraph, &mut shodo::LayoutContext::new());
    assert!(read(&store, body, "124").is_none());
    let record = read(&store, body, "123").unwrap();
    let mut sample = document.page_text_runs(&cascade, 0).pop().unwrap();
    let reference = sample.glyphs.clone();
    let width = sample.advance;
    assert!(!sample.decorations.is_empty());
    assert!(!record.apply(&mut sample, "123", width));
    sample.decorations.clear();
    sample.text = "abc";
    sample.glyphs.clear();
    assert!(!record.apply(&mut sample, "124", width));
    assert!(!record.apply(&mut sample, "123", width - 1.0));
    assert!(!record.apply(&mut sample, "123", f32::NAN));
    let face = sample.font.id;
    sample.font.id = FontId::from_blob_index(u64::MAX, 1);
    assert!(!record.apply(&mut sample, "123", width));
    sample.font.id = face;
    let size = sample.font_size;
    sample.font_size = size + 1.0;
    assert!(!record.apply(&mut sample, "123", width));
    sample.font_size = size;
    sample.variations.push(crate::FontVariation {
        tag: crate::Tag(*b"wght"),
        value: 550.0,
    });
    assert!(!record.apply(&mut sample, "123", width));
    sample.variations.clear();
    sample.synthesis.embolden = true;
    assert!(!record.apply(&mut sample, "123", width));
    sample.synthesis.embolden = false;
    sample.normalized_coords.push(1);
    assert!(!record.apply(&mut sample, "123", width));
    sample.normalized_coords.clear();
    assert_eq!(sample.text, "abc");
    assert!(sample.glyphs.is_empty());
    assert!(record.apply(&mut sample, "123", width));
    assert_eq!(sample.text, "123");
    assert_eq!(sample.glyphs, reference);
}

#[test]
fn capture_only_admits_short_printable_ascii() {
    for text in ["", "012345678901234567890", "日", "a\tb", "a\nb"] {
        assert!(!eligible_text(text));
    }
    assert!(eligible_text("01234567890123456789"));
}

#[test]
fn malformed_glyph_advances_decline_a_record() {
    let mut record = vec![0; record_len("x")];
    record[28] = 1;
    record[HEADER] = b'x';
    let start = HEADER + 5;
    for advance in [-1.0_f32, 1.0, f32::NAN, f32::INFINITY] {
        record[start..].copy_from_slice(&advance.to_le_bytes());
        assert!(decode(&record, "x").is_none());
    }
}
