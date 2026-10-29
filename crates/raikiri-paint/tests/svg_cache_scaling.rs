//! Opt-in production-path SVG cache probes. Run one case per process on one CPU.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

use anyrender::recording::RenderCommand;
use anyrender::{PaintScene, Scene};
use raikiri_html::{
    DocumentLayout, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources,
};
use raikiri_traits::{
    FetchOutcome, FetchedResource, ImagePixelSource, ImageRasterSize, ImageVectorRequest,
    NetworkError, NetworkProvider, Request,
};
use url::Url;

struct Allocator;
static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static COUNT: AtomicBool = AtomicBool::new(false);

fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes as i64, Ordering::Relaxed) + bytes as i64;
    if COUNT.load(Ordering::Relaxed) {
        PEAK.fetch_max(live, Ordering::Relaxed);
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

// SAFETY: All allocations are forwarded unchanged to System. Counters observe
// requested sizes only, and never dereference or alter allocated storage.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The allocator caller supplies a valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        // SAFETY: The caller supplies a live allocation and its matching layout.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live allocation and a valid new size.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
            allocated(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn phase<T>(name: &str, work: impl FnOnce() -> T) -> T {
    let before = LIVE.load(Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    COUNT.store(true, Ordering::Relaxed);
    let start = Instant::now();
    let result = work();
    let ns = start.elapsed().as_nanos();
    COUNT.store(false, Ordering::Relaxed);
    let calls = CALLS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    let live = LIVE.load(Ordering::Relaxed) - before;
    let peak = PEAK.load(Ordering::Relaxed) - before;
    println!(
        "SVG_PHASE name={name} ns={ns} allocations={calls} allocated_bytes={bytes} retained_bytes={live} peak_bytes={peak}"
    );
    result
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |sum, byte| {
        (sum ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn layout(html: &str, resources: &RenderResources<'_>) -> DocumentLayout {
    let doc = raikiri_html::parse_html_with_resources(html.as_bytes(), resources).unwrap();
    let LayoutStatus::Completed(result) = raikiri_html::layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .unwrap() else {
        panic!("layout aborted")
    };
    result
}

struct SvgProvider(Vec<u8>, AtomicU64);
impl NetworkProvider for SvgProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.1.fetch_add(1, Ordering::Relaxed);
        Ok(FetchOutcome::Body(
            FetchedResource::new(self.0.clone().into(), request.url)
                .content_type(Some("image/svg+xml".into())),
        ))
    }
}

fn source(uses: usize) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\" color=\"red\"><defs><path id=\"p\" d=\"M0 0H20V20H0Z\" fill=\"red\"/></defs>{}</svg>",
        "<use href=\"#p\"/>".repeat(uses)
    )
}

fn image_probe(kind: &str, count: usize) {
    let provider = SvgProvider(
        source(if kind == "source" { 500 } else { 1 }).into_bytes(),
        AtomicU64::new(0),
    );
    let resources = RenderResources::new().network_provider(&provider);
    let urls: Vec<_> = (0..count)
        .map(|at| Url::parse(&format!("https://svg.test/{at}.svg")).unwrap())
        .collect();
    let html = format!(
        "<!doctype html><style>body{{margin:0}}div{{height:4px;width:8px}}{}</style>{}",
        urls.iter()
            .enumerate()
            .map(|(at, url)| format!(".i{at}{{background-image:url('{url}')}}"))
            .collect::<String>(),
        (0..count)
            .map(|at| format!("<div class=i{at}></div>"))
            .collect::<String>()
    );
    phase("preload", || drop(layout(&html, &resources)));
    assert_eq!(provider.1.load(Ordering::Relaxed), count as u64);
    let request = ImageVectorRequest::new(ImageRasterSize {
        width: 20.0,
        height: 20.0,
    });
    let first = phase("vector_cold", || {
        urls.iter()
            .map(|url| {
                let scene = resources.get_vector(url, &request).expect("SVG scene");
                Arc::downgrade(&scene)
            })
            .collect::<Vec<_>>()
    });
    let mut hits = 0;
    phase("vector_warm_10", || {
        for _ in 0..10 {
            for (url, initial) in urls.iter().zip(&first) {
                let scene = resources.get_vector(url, &request).unwrap();
                hits += usize::from(initial.ptr_eq(&Arc::downgrade(&scene)));
                std::hint::black_box(scene);
            }
        }
    });
    let output = resources.get_vector(&urls[0], &request).unwrap();
    let digest = hash(format!("{output:?}").as_bytes());
    println!(
        "SVG_RESULT kind={kind} count={count} warm_hits={hits} warm_requests={} output_hash={digest:016x} fetches={}",
        count * 10,
        provider.1.load(Ordering::Relaxed)
    );
    drop(output);
    drop(first);
    phase("resource_drop", || drop(resources));
}

fn paint_probe(count: usize) {
    let resources = RenderResources::new();
    let svg = if count == 0 {
        String::new()
    } else {
        source(count)
    };
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0}}</style>{svg}"
    );
    let result = layout(&html, &resources);
    let page = result.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    let mut paint = || {
        scene.reset();
        raikiri_paint::paint_single_page(&mut scene, document, cascade, page_box).unwrap();
    };
    phase("paint_cold", &mut paint);
    phase("paint_warm_100", || {
        for _ in 0..100 {
            paint();
        }
    });
    let digest = scene.commands.iter().fold(0, |sum, command| match command {
        RenderCommand::Fill(fill) => match &fill.brush {
            anyrender::types::Paint::Image(image) => sum ^ hash(image.image.data.data()),
            _ => sum,
        },
        _ => sum,
    });
    if count > 0 {
        let node = (0..document.node_count())
            .find(|&id| document.get_node(id).unwrap().is_inline_svg_root())
            .unwrap();
        phase("serialize_100", || {
            for _ in 0..100 {
                std::hint::black_box(document.serialize_svg_subtree(node).unwrap());
            }
        });
        phase("style_collect_100", || {
            for _ in 0..100 {
                std::hint::black_box(
                    raikiri_dom::inline_svg_style::inline_svg_element_declarations(
                        document, cascade, node,
                    )
                    .unwrap(),
                );
            }
        });
        let serialized = document.serialize_svg_subtree(node).unwrap().unwrap();
        phase("parse_100", || {
            for _ in 0..100 {
                std::hint::black_box(
                    raikiri_svg::SvgDocument::parse(serialized.as_bytes()).unwrap(),
                );
            }
        });
        let parsed = raikiri_svg::SvgDocument::parse(serialized.as_bytes()).unwrap();
        phase("raster_100", || {
            for _ in 0..100 {
                std::hint::black_box(
                    parsed
                        .rasterize(
                            raikiri_svg::SvgViewport {
                                width: 20.0,
                                height: 20.0,
                            },
                            raikiri_svg::SvgRootStyle::default(),
                            None,
                        )
                        .unwrap(),
                );
            }
        });
    }
    println!(
        "SVG_RESULT kind=paint count={count} output_hash={digest:016x} commands={}",
        scene.commands.len()
    );
}

#[test]
#[ignore = "manual SVG performance probe; run alone with --nocapture --test-threads=1"]
fn svg_cache_scaling() {
    let case = std::env::var("SVG_CACHE_CASE").unwrap_or("paint:1".into());
    let (kind, count) = case.split_once(':').unwrap();
    let count = count.parse::<usize>().unwrap();
    assert!(count <= 500);
    // Initialize shared fonts and layout infrastructure outside the measurements.
    let resources = RenderResources::new();
    drop(layout("<!doctype html><body></body>", &resources));
    match kind {
        "source" | "scenes" => image_probe(kind, count),
        "paint" => paint_probe(count),
        _ => panic!("unknown probe case"),
    }
}
