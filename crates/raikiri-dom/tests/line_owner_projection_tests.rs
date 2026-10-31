//! Public explicit-page projection semantics and allocation measurements.
#[path = "support/allocation_tests.rs"]
mod support;
use raikiri_dom::{Document, PageSlice, PaintEvent};
use raikiri_html::{FontCollectionBuilder, RenderResources, parse_html_with_resources};
use raikiri_style::CascadeResult;
#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;
fn page() -> raikiri_traits::PageBox {
    let mut p = raikiri_traits::PageBox::new();
    p.width = 400.;
    p.height = 100.;
    p
}
fn input(html: &str, r: &RenderResources<'_>) -> (Document, CascadeResult) {
    let (mut parsed, cascade) = parse_html_with_resources(html.as_bytes(), r)
        .unwrap()
        .into_parts();
    parsed.dom.set_font_collection(r.inline_engine_fonts());
    parsed.dom.set_ifc_parallel_build(false);
    (parsed.dom, cascade)
}
struct Fixture {
    doc: Document,
    cascade: CascadeResult,
    slices: Vec<PageSlice>,
    p: usize,
    control: bool,
}
fn fixture(p: usize, control: bool, r: &RenderResources<'_>) -> Fixture {
    let h = if control { 10 } else { 100 * p };
    let html = format!(
        "<style>html,body,p{{margin:0;padding:0}}p{{font-family:Ahem;font-size:10px;line-height:{h}px;height:{}px}}p::before{{content:'X';background:red}}</style><p></p>",
        p * 100
    );
    let (mut doc, cascade) = input(&html, r);
    raikiri_dom::layout_single_page(&mut doc, &cascade, page()).unwrap();
    let roots: Vec<_> = (0..doc.node_count())
        .filter_map(|i| {
            doc.get_node(i)
                .unwrap()
                .ifc_lines()
                .map(|lines| (i, lines.len()))
        })
        .collect();
    assert_eq!(
        roots.iter().map(|(_, len)| len).sum::<usize>(),
        1,
        "one paragraph line: {roots:?}"
    );
    let root = roots[0].0;
    let line = &doc.get_node(root).unwrap().ifc_lines().unwrap()[0];
    assert_eq!(line.block_size(), h as f32, "real line height");
    assert_eq!(line.block_offset(), 0., "real line top");
    let slices = (0..p)
        .map(|i| PageSlice {
            page_index: i as u32,
            content_origin_y: i as f32 * 100.,
            page_name: None,
        })
        .collect();
    Fixture {
        doc,
        cascade,
        slices,
        p,
        control,
    }
}
fn project(f: &mut Fixture) -> usize {
    f.doc
        .project_pages(&f.cascade, page(), &f.slices, &[])
        .unwrap();
    f.p
}
fn validate(f: &mut Fixture) {
    let mut boxes = 0;
    let mut lines = std::collections::BTreeSet::new();
    let mut owning = Vec::new();
    let mut pages = 0;
    for i in 0..f.p {
        if f.doc.page_fragments(i as u32).count() > 0 {
            pages += 1;
        }
        let runs = f.doc.page_text_runs(&f.cascade, i as u32);
        let events = f
            .doc
            .page_paint_order_for_text_runs(&f.cascade, i as u32, &runs);
        for event in events {
            if let PaintEvent::GeneratedBox(b) = event {
                boxes += 1;
                lines.insert((b.line.root.0, b.line.index));
                owning.push(i);
            }
        }
    }
    assert_eq!(pages, f.p, "projected nonempty page count");
    assert_eq!(boxes, if f.control { 1 } else { f.p }, "generated boxes");
    assert_eq!(lines.len(), 1, "one projected line identity");
    if !f.control {
        assert_eq!(owning, (0..f.p).collect::<Vec<_>>());
    }
    assert_eq!(
        owning,
        if f.control {
            vec![0]
        } else {
            (0..f.p).collect::<Vec<_>>()
        }
    );
}

#[test]
fn a_tall_generated_line_keeps_its_placements_across_explicit_pages() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    for pages in [256, 512, 1024, 2048, 4096] {
        for control in [false, true] {
            let mut times = Vec::new();
            for sample in 0..6 {
                let mut f = fixture(pages, control, &resources);
                let start = std::time::Instant::now();
                project(&mut f);
                let elapsed = start.elapsed();
                if sample > 0 {
                    times.push(elapsed);
                }
            }
            times.sort();
            let mut f = fixture(pages, control, &resources);
            let (actual, allocation) = support::allocated_by(|| project(&mut f));
            assert_eq!(actual, pages);
            validate(&mut f);
            eprintln!(
                "line-owner projection: pages={pages}, control={control}, median_ms={:.6}, {allocation:?}",
                times[2].as_secs_f64() * 1000.
            );
        }
    }
}
