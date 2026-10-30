//! Running-layout admission and hit lookup through the public document API.
//! Live requested bytes exclude allocator overhead and are not RSS.

// cov:ignore: standalone benchmark is not instrumented by workspace coverage.
mod measurement {
    use raikiri_html::{
        FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
        RenderResources, layout, parse_html_with_resources,
    };
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        hint::black_box,
        sync::atomic::{AtomicIsize, Ordering::Relaxed},
        time::Instant,
    };
    struct Counting;
    static LIVE: AtomicIsize = AtomicIsize::new(0);
    // SAFETY: all allocation contracts are forwarded unchanged to System; atomics allocate nothing.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            // SAFETY: the caller supplies a valid allocation layout.
            let p = unsafe { System.alloc(l) };
            if !p.is_null() {
                LIVE.fetch_add(l.size() as isize, Relaxed);
            }
            p
        }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            // SAFETY: the caller supplies a valid allocation layout.
            let p = unsafe { System.alloc_zeroed(l) };
            if !p.is_null() {
                LIVE.fetch_add(l.size() as isize, Relaxed);
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            // SAFETY: the caller supplies the original allocation and its matching layout.
            unsafe { System.dealloc(p, l) };
            LIVE.fetch_sub(l.size() as isize, Relaxed);
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            // SAFETY: the caller upholds the realloc contract; a failed realloc keeps the old allocation.
            let q = unsafe { System.realloc(p, l, n) };
            if !q.is_null() {
                LIVE.fetch_add(n as isize - l.size() as isize, Relaxed);
            }
            q
        }
    }
    #[global_allocator]
    static ALLOC: Counting = Counting;
    pub fn run() {
        let after = !std::env::args().any(|arg| arg == "--before");
        let fonts = FontCollectionBuilder::new()
            .font_bytes(
                "Ahem",
                include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
            )
            .build()
            .unwrap();
        let resources = RenderResources::new().fonts(fonts);
        let html = format!(
            "<!doctype html><html><head><style>body{{font-family:Ahem;font-size:10px}}header{{position:running(h)}}</style></head><body><header id=h>{}</header><p>a</p></body></html>",
            "<span>a</span>".repeat(16)
        );
        for limit in [128usize, 1024] {
            for n in [1usize, 16, 64, 256, 1024] {
                let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
                let node = parsed
                    .dom()
                    .preorder(parsed.dom().root_index())
                    .find(|&id| parsed.dom().element_attribute(id, "id") == Some("h"))
                    .unwrap();
                let LayoutStatus::Completed(out) = layout(
                    &parsed,
                    PageDefaults::default(),
                    LayoutConfig::default(),
                    LayoutOptions::new()
                        .resources(&resources)
                        .max_running_layout_entries(limit),
                )
                .unwrap() else {
                    panic!("unexpected abort")
                };
                let node = raikiri_traits::NodeId::new(node as u64);
                let original =
                    out.layout_running_element(node, 200.).unwrap().unwrap() as *const _ as usize;
                let baseline = LIVE.load(Relaxed);
                let mut admitted = 1;
                for i in 1..n {
                    let width = 200. + i as f32 / 16.;
                    match out.layout_running_element(node, width) {
                        Ok(Some(value)) => {
                            black_box(value);
                            admitted += 1;
                        }
                        Err(raikiri_traits::RenderError::LimitExceeded {
                            kind: raikiri_traits::LimitKind::RunningLayoutEntries,
                            limit: configured,
                            actual,
                        }) if after
                            && admitted == limit
                            && configured == limit as u64
                            && actual == limit as u64 + 1 => {}
                        other => panic!("unexpected admission: {other:?}"),
                    }
                }
                assert_eq!(admitted, if after { n.min(limit) } else { n });
                let retained = LIVE.load(Relaxed) - baseline;
                assert_eq!(
                    original,
                    out.layout_running_element(node, 200.).unwrap().unwrap() as *const _ as usize
                );
                let last = 200. + (admitted - 1) as f32 / 16.;
                let mut first_ms = Vec::new();
                let mut last_ms = Vec::new();
                for _ in 0..3 {
                    for (width, times) in [(200., &mut first_ms), (last, &mut last_ms)] {
                        let live_before = LIVE.load(Relaxed);
                        let t = Instant::now();
                        for _ in 0..10000 {
                            black_box(
                                out.layout_running_element(node, black_box(width))
                                    .unwrap()
                                    .unwrap(),
                            );
                        }
                        let elapsed = t.elapsed().as_secs_f64() * 1000.;
                        assert_eq!(LIVE.load(Relaxed), live_before);
                        times.push(elapsed);
                    }
                }
                first_ms.sort_by(f64::total_cmp);
                last_ms.sort_by(f64::total_cmp);
                raikiri_html::begin_running_cache_diagnostics();
                for _ in 0..10000 {
                    black_box(
                        out.layout_running_element(node, black_box(last))
                            .unwrap()
                            .unwrap(),
                    );
                }
                let work = raikiri_html::finish_running_cache_diagnostics();
                assert_eq!(work.make_calls, 0);
                assert_eq!(work.admitted_entries, 0);
                if after {
                    assert!(work.width_comparisons <= 33 * 10000);
                }
                for i in 0..admitted {
                    let value = out
                        .layout_running_element(node, 200. + i as f32 / 16.)
                        .unwrap()
                        .unwrap();
                    let text = value
                        .page()
                        .text_runs()
                        .into_iter()
                        .map(|run| run.text)
                        .collect::<String>();
                    assert_eq!(text, "a".repeat(16));
                }
                println!(
                    "after={after} limit={limit} widths={n} admitted={admitted} added_live_bytes={retained} first_raw_ms={first_ms:?} last_raw_ms={last_ms:?} work={work:?}"
                );
            }
        }
    }
}
fn main() {
    measurement::run();
}
