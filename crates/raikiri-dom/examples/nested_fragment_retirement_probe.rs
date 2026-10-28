//! Manual fragment-retirement scaling probe; run release without concurrent builds.

use std::hint::black_box;
use std::time::Instant;

use raikiri_dom::Document;
use raikiri_style::{Origin, RuleTree};

fn fixture(groups: usize, nested: bool) -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    for _ in 0..groups {
        let outer = doc.append_element(
            Some(body),
            "div",
            Some("display:block;width:100px;columns:4;column-gap:0"),
        );
        let inner = doc.append_element(
            Some(outer),
            "div",
            Some(if nested {
                "display:block;columns:2;column-gap:0"
            } else {
                "display:block"
            }),
        );
        let parent = if nested {
            inner
        } else {
            doc.append_element(Some(inner), "div", Some("display:block"))
        };
        doc.append_element(
            Some(parent),
            "div",
            Some(if nested {
                "display:block;width:12.5px;height:400px"
            } else {
                "display:block;width:10px;height:40px"
            }),
        );
    }
    doc.mark_in_document_flags();
    doc
}

fn main() {
    for nested in [true, false] {
        for groups in [16, 32, 64, 128, 256, 512, 1024, 2048] {
            for sample in 0..5 {
                let mut doc = fixture(groups, nested);
                let mut rules = RuleTree::empty();
                rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
                let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
                let mut page = raikiri_traits::PageBox::new();
                page.width = 800.0;
                page.height = 600.0;
                let start = Instant::now();
                raikiri_dom::layout_single_page(&mut doc, &cascade, page).unwrap();
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(doc.node_count(), 3 + groups * if nested { 3 } else { 4 });
                assert!(!doc.layout_fragments().is_empty());
                // All fields participate; formatting is outside the layout timing.
                let mut signature = 0_u64;
                for byte in format!("{:?}", doc.layout_fragments()).bytes() {
                    signature = signature
                        .wrapping_mul(1_099_511_628_211)
                        .wrapping_add(u64::from(byte));
                }
                println!(
                    "retirement nested={nested} groups={groups} sample={sample} nodes={} fragments={} signature={signature} ms={ms:.6}",
                    doc.node_count(),
                    doc.layout_fragments().len()
                );
                black_box(doc);
            }
        }
    }
}
