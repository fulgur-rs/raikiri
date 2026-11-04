use super::*;

fn add(doc: &mut crate::Document, parent: usize, tag: &str) -> usize {
    doc.append_element(
        Some(parent),
        tag,
        Some(match tag {
            "ruby" => "display:ruby",
            "rtc" => "display:ruby-text-container",
            _ => "display:ruby-text;font-size:5px",
        }),
    )
}

#[test]
fn real_content_disambiguates_nested_annotations_with_the_same_triple() {
    let mut ids = (0, 0);
    let (root, lines) = root_and_lines_of("line-height:20px", 800.0, |doc, root| {
        let outer = add(doc, root, "ruby");
        let inner = add(doc, outer, "ruby");
        let inner_rtc = add(doc, inner, "rtc");
        doc.append_text(inner_rtc, "x");
        let outer_rtc = add(doc, outer, "rtc");
        doc.append_element(
            Some(outer_rtc),
            "span",
            Some("display:inline-block;width:10px;height:10px"),
        );
        ids = (inner_rtc, outer_rtc);
    });
    let annotations: Vec<_> = lines[0].ruby_annotations().collect();
    assert_eq!(annotations.len(), 2);
    let identity = |a: shodo::RubyAnnotationView<'_>| (a.container(), a.level(), a.node());
    assert_eq!(identity(annotations[0]), identity(annotations[1]));
    let owner = |annotation: shodo::RubyAnnotationView<'_>| {
        let fragments: Vec<_> = annotation.line().fragments().collect();
        super::super::rtc::owner(annotation, &fragments, &root.ruby_containers)
    };
    assert_eq!(owner(annotations[0]), Some(ids.0));
    assert_eq!(owner(annotations[1]), Some(ids.1));
}

#[test]
fn nested_annotation_content_keeps_its_own_container_owner() {
    let mut ids = (0, 0);
    let (root, lines) = root_and_lines_of("line-height:20px", 800.0, |doc, root| {
        let outer = add(doc, root, "ruby");
        doc.append_text(outer, "a");
        let outer_rtc = add(doc, outer, "rtc");
        let inner = add(doc, outer_rtc, "ruby");
        doc.append_text(inner, "b");
        let inner_rtc = add(doc, inner, "rtc");
        doc.append_text(inner_rtc, "x");
        ids = (outer_rtc, inner_rtc);
    });
    let outer = lines[0].ruby_annotations().next().unwrap();
    let inner = outer.line().ruby_annotations().next().unwrap();
    for (annotation, expected) in [(outer, ids.0), (inner, ids.1)] {
        let fragments: Vec<_> = annotation.line().fragments().collect();
        assert_eq!(
            super::super::rtc::owner(annotation, &fragments, &root.ruby_containers),
            Some(expected)
        );
    }
}

#[test]
fn independent_inter_character_containers_take_linear_piece_work() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    use crate::layout::{layout_single_page, test_support::page_box_800x600};

    let mut inspected = Vec::new();
    for count in [32, 64, 128] {
        let fixture =
            block_fixture_with_ahem("white-space:nowrap;line-height:20px", |doc, root| {
                for _ in 0..count {
                    let ruby = add(doc, root, "ruby");
                    doc.append_text(ruby, "a");
                    let rtc = doc.append_element(
                        Some(ruby),
                        "rtc",
                        Some("display:ruby-text-container;ruby-position:inter-character"),
                    );
                    let rt = add(doc, rtc, "rt");
                    doc.append_text(rt, "x");
                }
            });
        let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
        layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        work::take();
        let pieces = doc.nodes[root].ifc_inline_boxes().unwrap();
        let work = work::take();
        assert_eq!(
            pieces
                .iter()
                .filter(|piece| doc.nodes[piece.node].tag_name() == Some("rtc"))
                .count(),
            count
        );
        inspected.push(work.inspected);
    }
    eprintln!("RTC piece inspections {inspected:?}");
    for pair in inspected.windows(2) {
        assert!(
            pair[1] <= 3 * pair[0],
            "doubling independent RTCs must stay linear: {inspected:?}"
        );
    }
}
