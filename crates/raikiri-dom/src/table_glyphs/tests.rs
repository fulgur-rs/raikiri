use super::*;

fn test_store() -> store::Store {
    store::Store::new(
        tempfile::tempfile().unwrap(),
        tempfile::tempfile().unwrap(),
        4096,
    )
}

fn test_fonts() -> shodo::font::FontCollection {
    crate::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace"
    )))
    .unwrap()
}

#[test]
fn measured_glyphs_match_projected_text_and_require_compatible_sample() {
    let mut document = crate::Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        Some("display:block;font:10px Ahem;text-decoration:underline;color:maroon;text-shadow:1px 2px red"),
    );
    document.append_text(body, "123");
    let fonts = test_fonts();
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
    let store = test_store();
    let paragraph = &document
        .get_node(body)
        .unwrap()
        .ifc
        .as_ref()
        .unwrap()
        .paragraph;
    let mut cx = shodo::LayoutContext::new();
    let options = shodo::style::LineOptions::default();
    capture(&store, body, " 123 ", paragraph, &options, &mut cx);
    assert!(read(&store, body, "123").is_none());
    let justify = shodo::style::LineOptions {
        text_align_last: shodo::style::TextAlignLast::Justify,
        ..options
    };
    capture(&store, body, "123", paragraph, &justify, &mut cx);
    assert!(read(&store, body, "123").is_none());
    let centered = shodo::style::LineOptions {
        text_align: shodo::style::TextAlign::Center,
        ..options
    };
    capture(&store, body, "123", paragraph, &centered, &mut cx);
    assert!(read(&store, body, "123").is_none());
    capture(&store, body, "123", paragraph, &options, &mut cx);
    assert!(read(&store, body, "124").is_none());
    let record = read(&store, body, "123").unwrap();
    let mut sample = document.page_text_runs(&cascade, 0).pop().unwrap();
    let reference = sample.glyphs.clone();
    let width = sample.advance;
    let color = sample.color;
    let shadows = sample.shadows.clone();
    assert_eq!(shadows.len(), 1);
    let transform = Some(raikiri_traits::PaintTransform::translate(3.0, 7.0));
    sample.transform = transform;
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
    sample.text_offset = 1;
    assert!(!record.apply(&mut sample, "123", width));
    sample.text_offset = 0;
    assert_eq!(sample.text, "abc");
    assert!(sample.glyphs.is_empty());
    assert!(record.apply(&mut sample, "123", width));
    assert_eq!(sample.text, "123");
    assert_eq!(sample.glyphs, reference);
    assert_eq!(sample.color, color);
    assert_eq!(sample.shadows, shadows);
    assert_eq!(sample.transform, transform);
}

#[test]
fn read_only_admits_short_printable_ascii() {
    let store = test_store();
    for (id, (text, readable)) in [
        ("", false),
        ("日", false),
        ("a\tb", false),
        ("a\nb", false),
        ("01234567890123456789", true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut record = vec![0; record_len(text)];
        record[28] = text.len() as u8;
        record[HEADER..HEADER + text.len()].copy_from_slice(text.as_bytes());
        store.write(id, &record);
        assert_eq!(read(&store, id, text).is_some(), readable, "{text:?}");
    }
    assert!(read(&store, 0, "012345678901234567890").is_none());
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

#[test]
fn capture_declines_paragraphs_that_need_full_line_projection() {
    use shodo::geometry::Direction;
    use shodo::node::{InlineEdges, NodeId, Sides, TextSource};
    use shodo::style::ParagraphStyle;

    let fonts = test_fonts();
    let store = test_store();
    let mut cx = shodo::LayoutContext::new();
    let style = ParagraphStyle::default();
    let builder = |style: &ParagraphStyle| shodo::ParagraphBuilder::new(style, &Default::default());
    let source = TextSource::Dom {
        node: NodeId(1),
        offset: 0,
    };

    let empty = builder(&style).build(&mut cx, &fonts).unwrap();
    let result = empty.next_line(
        &mut cx,
        empty.start_token(),
        &Default::default(),
        &shodo::LineConstraint::new(100.0),
        &shodo::AtomicSizes::EMPTY,
    );
    assert!(matches!(result, shodo::LineResult::Done));
    capture_line(&store, 0, empty.text(), result);
    assert!(!store.read(0, &mut [0; HEADER]));

    let mut rtl = builder(&ParagraphStyle {
        direction: Direction::Rtl,
        ..style.clone()
    });
    rtl.push_text(source, "a");

    let mut boxed = builder(&style);
    let edges = InlineEdges {
        padding: Sides {
            inline_start: 4.0,
            ..Default::default()
        },
        ..Default::default()
    };
    boxed
        .open_inline(NodeId(2), &style.root, edges)
        .push_text(source, "a")
        .close_inline();

    let mut multi = builder(&style);
    multi.push_text(source, "a");
    let larger = shodo::style::InlineStyle {
        font_size: 32.0,
        ..style.root.clone()
    };
    multi
        .open_inline(NodeId(2), &larger, Default::default())
        .push_text(
            TextSource::Dom {
                node: NodeId(3),
                offset: 0,
            },
            "b",
        )
        .close_inline();

    for (name, input) in [
        ("RTL", rtl),
        ("inline box", boxed),
        ("multiple runs", multi),
    ] {
        let paragraph = input.build(&mut cx, &fonts).unwrap();
        let text = paragraph.text();
        assert!(eligible_text(text), "{name}");
        capture(&store, 0, text, &paragraph, &Default::default(), &mut cx);
        assert!(read(&store, 0, text).is_none(), "{name}");
    }
}
