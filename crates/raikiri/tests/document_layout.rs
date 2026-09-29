//! Layout coverage for page geometry, resources, and links.

use raikiri::{
    DocumentLayout, FragmentKind, IntrinsicBox, LayoutConfig, LayoutOptions, LayoutStatus, PageBox,
    PageDefaults, RenderResources, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic,
    ResolverError, ResolverRequest, layout, parse_html,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoopResolver;

impl ReplacedResolver for NoopResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        unreachable!("test documents contain no replaced elements")
    }
}

struct FixedIntrinsicResolver {
    width: f32,
    calls: AtomicUsize,
}

impl ReplacedResolver for FixedIntrinsicResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(self.width, 40.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

fn defaults(width: f32, height: f32) -> PageDefaults {
    let mut defaults = PageDefaults::default();
    let mut page_box = PageBox::new();
    page_box.width = width;
    page_box.height = height;
    defaults.page_box = page_box;
    defaults
}

#[test]
fn forced_break_pages_are_returned_in_order() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:40px">first</div><div style="break-before:page;height:10px">second</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 50.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );

    assert_eq!(result.pages().collect::<Vec<_>>().len(), 2);
    assert_eq!(
        result
            .pages()
            .collect::<Vec<_>>()
            .iter()
            .map(|page| page.index())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(result.page_count(), 2);
}

#[test]
fn layout_returns_page_local_links_with_fragment_geometry() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br##"<html><body><a href="#target">link</a><div style="break-before:page">second</div></body></html>"##[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 50.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );

    assert_eq!(result.pages().collect::<Vec<_>>().len(), 2);
    let page = result.page(0).unwrap();
    let links: Vec<_> = page.links().collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "#target");
    assert!(
        page.fragments()
            .any(|fragment| fragment.kind() == FragmentKind::Text
                && links[0].quads.contains(&fragment.rect()))
    );
}

#[test]
fn layout_resolves_page_geometry_from_page_contexts() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 5px; }
        @page :first { size: 100px 120px; margin: 10px; }
        @page :right { padding: 3px; }
        @page :left { size: 120px 100px; margin: 7px; }
        @page wide { size: 200px 200px; margin: 20px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:90px">first</div><div style="page:wide;break-before:page;height:10px">wide</div><div style="height:180px">tail</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );
    assert!(result.pages().collect::<Vec<_>>().len() >= 2);
    let first = result.page(0).unwrap();
    assert_eq!(first.geometry().page_box.height, 120.0);
    assert_eq!(first.geometry().margins.top, 10.0);
    assert_eq!(
        (first.geometry().content_box.x - first.geometry().margins.left),
        3.0
    );

    let wide = result
        .pages()
        .find(|page| page.name() == Some("wide"))
        .expect("named page geometry");
    assert_eq!(wide.geometry().page_box.width, 200.0);
    assert_eq!(wide.geometry().page_box.height, 200.0);
    assert_eq!(wide.geometry().margins.left, 20.0);
    assert_eq!(
        (wide.geometry().content_box.x - wide.geometry().margins.left),
        0.0
    );

    let left = result
        .pages()
        .find(|page| page.geometry().page_box.width == 120.0)
        .expect("left-page geometry");
    assert_eq!(left.geometry().margins.left, 7.0);
    assert_eq!(left.geometry().content_box.x, 7.0);
    assert!(
        result
            .pages()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pages| pages[0].geometry().page_box != pages[1].geometry().page_box)
    );
}

#[test]
fn layout_resolves_named_first_page_before_layout() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 5px; }
        @page cover { size: 130px 140px; margin: 11px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="page:cover;height:10px">cover</div><div style="break-before:page;height:10px">next</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );

    let first = result.pages().next().expect("first page");
    assert_eq!(first.name(), Some("cover"));
    assert_eq!(first.geometry().page_box.width, 130.0);
    assert_eq!(first.geometry().page_box.height, 140.0);
    assert_eq!(first.geometry().margins.left, 11.0);
}

#[test]
fn layout_resolves_nested_grid_page_before_layout() {
    let stylesheet = r#"
        @page wide { size: 200px 300px; margin: 5px; }
        @page narrow { size: 120px 180px; margin: 12px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100%;grid-template-rows:auto auto"><div style="grid-row:2;order:0;page:wide;height:10px">wide</div><div style="grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">narrow page text</div></div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );

    let first = result.pages().next().expect("first page");
    assert_eq!(first.name(), Some("narrow"));
    assert_eq!(
        (
            first.geometry().page_box.width,
            first.geometry().page_box.height
        ),
        (120.0, 180.0)
    );
    assert_eq!(first.geometry().margins.left, 12.0);
}

#[test]
fn layout_preflight_uses_resolved_image_size_for_grid_page_selection() {
    let html = &br#"<!doctype html><style>
        @page wide { size:200px 300px; margin:5px }
        @page narrow { size:120px 180px; margin:12px }
        body { margin:0 }
        .flex { display:flex; width:300px }
        .grid { display:grid; order:0; flex:1 1 0; min-width:0;
                grid-template-columns:repeat(auto-fit,minmax(100px,1fr));
                grid-template-rows:auto auto }
        img { order:1; flex:0 0 auto }
    </style><body><div class="flex"><div class="grid">
        <div style="display:block;grid-row:2;order:0;page:wide;height:10px">wide</div>
        <div style="display:block;grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">narrow page text</div>
    </div><img src="image.png"></div></body>"#[..];
    let base_url = raikiri::Url::parse("https://example.test/assets/").expect("base URL");
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(html, &options).expect("parse");
    assert!((0..doc.dom().node_count()).any(|node_id| {
        doc.dom()
            .get_node(node_id)
            .is_some_and(|node| node.tag_name() == Some("img") && node.attribute("src").is_some())
    }));

    let small_resolver = FixedIntrinsicResolver {
        width: 50.0,
        calls: AtomicUsize::new(0),
    };
    let small_resources = RenderResources::new()
        .base_url(base_url.clone())
        .replaced_resolver(&small_resolver);
    let small_result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&small_resources),
        )
        .expect("small-image render"),
    );

    let large_resolver = FixedIntrinsicResolver {
        width: 250.0,
        calls: AtomicUsize::new(0),
    };
    let large_resources = RenderResources::new()
        .base_url(base_url)
        .replaced_resolver(&large_resolver);
    let large_result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&large_resources),
        )
        .expect("large-image render"),
    );

    assert!(small_resolver.calls.load(Ordering::Relaxed) >= 2);
    assert!(large_resolver.calls.load(Ordering::Relaxed) >= 3);
    let small_first = small_result.pages().next().expect("small first page");
    let large_first = large_result.pages().next().expect("large first page");
    assert_eq!(small_first.name(), Some("wide"));
    assert_eq!(large_first.name(), Some("narrow"));
    assert_eq!(
        (
            small_first.geometry().page_box.width,
            small_first.geometry().page_box.height
        ),
        (200.0, 300.0)
    );
    assert_eq!(
        (
            large_first.geometry().page_box.width,
            large_first.geometry().page_box.height
        ),
        (120.0, 180.0)
    );
}

#[test]
fn layout_relayouts_until_page_geometry_converges() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 0; }
        @page :left { size: 100px 240px; margin: 0; }
        @page wide { size: 100px 180px; margin: 0; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:200px">first</div><div style="page:wide;break-before:page;height:10px">wide</div><div style="height:300px">tail</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let result = completed(
        layout(
            &doc,
            defaults(100.0, 100.0),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("render"),
    );

    assert_eq!(result.page_count(), 4);
    assert_eq!(result.pages().collect::<Vec<_>>().len(), 4);
    assert_eq!(
        result.pages().collect::<Vec<_>>()[0]
            .geometry()
            .page_box
            .height,
        100.0
    );
    assert_eq!(
        result.pages().collect::<Vec<_>>()[1]
            .geometry()
            .page_box
            .height,
        180.0
    );
    assert_eq!(
        result.pages().collect::<Vec<_>>()[2]
            .geometry()
            .page_box
            .height,
        100.0
    );
    assert_eq!(
        result.pages().collect::<Vec<_>>()[3]
            .geometry()
            .page_box
            .height,
        240.0
    );
    assert_eq!(result.pages().collect::<Vec<_>>()[1].name(), Some("wide"));

    // Every emitted item must fit the resolved page-local geometry. Before the
    // schedule refresh, page 1 used the 240px item projection with a 180px
    // `wide` page metadata record.
    for page in result.pages() {
        assert!(page.fragments().all(|item| {
            item.rect().y >= -0.001
                && item.rect().y + item.rect().height <= page.geometry().page_box.height + 0.001
        }));
    }
}

fn completed(status: LayoutStatus) -> DocumentLayout {
    match status {
        LayoutStatus::Completed(result) => result,
        _ => panic!("expected completed layout"),
    }
}
