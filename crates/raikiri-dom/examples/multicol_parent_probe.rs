//! Manual wrapped-column lookup probe. Run the release binary without concurrent builds.

use std::hint::black_box;
use std::time::Instant;

use raikiri_dom::Document;
use raikiri_style::{Origin, RuleTree};

fn fixture(columns: usize, variant: &str) -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let extra = match variant {
        "rtl" => ";direction:rtl",
        "overflow" => ";height:9px;column-fill:auto",
        _ => "",
    };
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;width:{}px;columns:{columns};column-gap:0{extra}",
            columns * 10
        )),
    );
    let wrapper_style = match variant {
        "slice" => {
            "display:block;padding-top:2px;padding-bottom:3px;border-top:1px solid;border-bottom:1px solid;box-decoration-break:slice"
        }
        "clone" => {
            "display:block;padding-top:2px;padding-bottom:3px;border-top:1px solid;border-bottom:1px solid;box-decoration-break:clone"
        }
        _ => "display:block",
    };
    let wrapper = doc.append_element(Some(container), "div", Some(wrapper_style));
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

fn main() {
    for variant in ["plain", "rtl", "overflow", "slice", "clone"] {
        for columns in [128, 256, 512, 1024, 2048, 4096, 8192] {
            for sample in 0..5 {
                let mut doc = fixture(columns, variant);
                let mut rules = RuleTree::empty();
                rules.add_stylesheet("html,body{display:block;margin:0}", Origin::Author);
                let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
                let mut page = raikiri_traits::PageBox::new();
                page.width = columns as f32 * 10.0 + 20.0;
                page.height = 600.0;
                let start = Instant::now();
                raikiri_dom::layout_single_page(&mut doc, &cascade, page).unwrap();
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(doc.node_count(), 7);
                let fragments = doc.layout_fragments();
                assert!(!fragments.is_empty());
                // Debug includes every field; this signature checks more than geometry alone.
                let mut signature = 0_u64;
                for byte in format!("{fragments:?}").bytes() {
                    signature = signature
                        .wrapping_mul(1_099_511_628_211)
                        .wrapping_add(u64::from(byte));
                }
                println!(
                    "wrapped variant={variant} columns={columns} sample={sample} nodes={} fragments={} signature={signature} ms={ms:.6}",
                    doc.node_count(),
                    fragments.len()
                );
                black_box(doc);
            }
        }
    }
}
