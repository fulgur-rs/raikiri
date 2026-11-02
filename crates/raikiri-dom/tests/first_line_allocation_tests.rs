//! Public allocation guard for decorated first-line inheritance chains.
#[path = "support/allocation_tests.rs"]
mod support;
use raikiri_dom::{Document, layout_single_page};
use raikiri_html::{FontCollectionBuilder, RenderResources, parse_html_with_resources};
use raikiri_style::{CascadeResult, PseudoElem, StyleNodeId};
#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;
fn page() -> raikiri_traits::PageBox {
    let mut p = raikiri_traits::PageBox::new();
    p.width = 800.;
    p.height = 600.;
    p
}
fn input(html: &str, resources: &RenderResources<'_>) -> (Document, CascadeResult) {
    let (mut parsed, cascade) = parse_html_with_resources(html.as_bytes(), resources)
        .unwrap()
        .into_parts();
    parsed
        .dom
        .set_font_collection(resources.inline_engine_fonts());
    parsed.dom.set_ifc_parallel_build(false);
    (parsed.dom, cascade)
}
struct Fixture {
    doc: Document,
    cascade: CascadeResult,
    origins: Vec<StyleNodeId>,
    paragraph: usize,
    text: usize,
}
fn fixture(n: usize, variant: &str, resources: &raikiri_html::RenderResources<'_>) -> Fixture {
    assert!(n > 0);
    let pseudo = match variant {
        "all" => "div::first-line{color:rgb(1,2,3);text-decoration:underline}",
        "nodec" => "div::first-line{color:rgb(1,2,3)}",
        "fixed" => "body>div::first-line{color:rgb(1,2,3);text-decoration:underline}",
        _ => panic!("unknown variant: {variant}"),
    };
    let html = format!(
        "<!doctype html><html><head><style>html,body,div{{display:block;margin:0;padding:0}}body{{font-family:Ahem;font-size:10px;line-height:10px}}{pseudo}</style></head><body></body></html>"
    );
    let (mut doc, _) = input(&html, resources);
    let body = (0..doc.node_count())
        .find(|&id| doc.get_node(id).unwrap().tag_name() == Some("body"))
        .unwrap();
    let mut parent = body;
    let mut ids = Vec::with_capacity(n);
    for _ in 0..n {
        parent = doc.append_element(Some(parent), "div", Some("display:block"));
        ids.push(parent);
    }
    let text = doc.append_text(parent, "A");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let origins: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&id| {
            cascade
                .pseudo
                .contains_key(&(StyleNodeId::new(id as u64), PseudoElem::FirstLine))
        })
        .map(|id| StyleNodeId::new(id as u64))
        .collect();
    assert_eq!(origins.len(), if variant == "fixed" { 1 } else { n });
    Fixture {
        doc,
        cascade,
        origins,
        paragraph: parent,
        text,
    }
}
fn validate(f: &Fixture, variant: &str) -> usize {
    let lines = f.doc.ifc_text_lines(f.text).expect("A has glyphs");
    assert_eq!(lines.root, f.paragraph);
    assert_eq!(lines.lines.len(), 1);
    assert_eq!(lines.lines[0].bottom - lines.lines[0].top, 10.);
    assert_eq!(
        f.doc.get_node(f.paragraph).unwrap().layout().size.height,
        10.
    );
    let decorating = f
        .doc
        .get_node(f.paragraph)
        .unwrap()
        .ifc_first_line_decorating();
    assert_eq!(
        decorating.len(),
        if variant == "nodec" {
            0
        } else {
            f.origins.len()
        }
    );
    for ((id, cv), expected) in decorating.iter().zip(&f.origins) {
        assert_eq!(*id as u64, expected.0);
        assert_eq!((cv.color.r, cv.color.g, cv.color.b), (1, 2, 3));
        assert_eq!(cv.font_size.px(), 10.);
    }
    decorating.len() + lines.lines.len()
}

#[test]
fn first_line_decorations_reuse_origin_prefix_values() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let mut warmup = fixture(1, "all", &resources);
    layout_single_page(&mut warmup.doc, &warmup.cascade, page()).unwrap();
    validate(&warmup, "all");
    let mut readings = Vec::new();
    for depth in [16, 32, 64, 128] {
        for variant in ["all", "nodec", "fixed"] {
            let mut f = fixture(depth, variant, &resources);
            let (result, allocation) =
                support::allocated_by(|| layout_single_page(&mut f.doc, &f.cascade, page()));
            result.unwrap();
            validate(&f, variant);
            eprintln!("first-line layout: depth={depth}, variant={variant}, {allocation:?}");
            readings.push((depth, variant, allocation.bytes));
        }
    }
    for (depth, variant, bytes) in readings {
        assert!(
            bytes < 512 * 1024 + depth * 64 * 1024,
            "depth={depth},variant={variant},bytes={bytes}"
        );
    }
}
