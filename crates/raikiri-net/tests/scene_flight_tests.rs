#![allow(unsafe_code)]
//! Public concurrent SVG scene identity and allocation regression coverage.

use raikiri_net::ImageResolver;
use raikiri_traits::{
    FetchOutcome, FetchedResource, ImagePixelSource, ImageRasterSize, ImageVectorRequest,
    NetworkError, NetworkProvider, ReplacedResolver, Request, ResolverRequest,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use url::Url;

static COUNTING: AtomicBool = AtomicBool::new(false);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

fn record(bytes: usize) {
    if COUNTING.load(Ordering::Relaxed) {
        BYTES.fetch_add(bytes, Ordering::Relaxed);
        CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: All operations forward the original pointer and layout to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            record(layout.size());
        }
        result
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies the live pointer, layout and new size.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            record(size);
        }
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies the live pointer and its allocation layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct StopCounting;
impl Drop for StopCounting {
    fn drop(&mut self) {
        COUNTING.store(false, Ordering::SeqCst);
    }
}

struct SvgProvider {
    bytes: Arc<[u8]>,
    calls: Arc<AtomicUsize>,
}

impl NetworkProvider for SvgProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(FetchOutcome::Body(
            FetchedResource::new(self.bytes.to_vec().into(), request.url)
                .content_type(Some("image/svg+xml".into())),
        ))
    }
}

#[test]
fn public_concurrent_vector_requests_share_one_retained_scene_and_build_allocation() {
    let mut failures = Vec::new();
    for rectangles in [1000, 8000] {
        let mut svg =
            String::from("<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>");
        for index in 0..rectangles {
            svg.push_str(&format!(
                "<rect x='{}' y='{}' width='1' height='1' fill='red'/>",
                index % 100,
                index / 100
            ));
        }
        svg.push_str("</svg>");
        let bytes: Arc<[u8]> = svg.into_bytes().into();
        let mut serial_bytes = 0;
        for warm in [false, true] {
            for workers in [1, 4, 8] {
                let fetches = Arc::new(AtomicUsize::new(0));
                let resolver = ImageResolver::new(SvgProvider {
                    bytes: bytes.clone(),
                    calls: fetches.clone(),
                });
                let url = Url::parse("https://images.test/scene.svg").unwrap();
                resolver.resolve(ResolverRequest::new(&url)).unwrap();
                let request = ImageVectorRequest::new(ImageRasterSize {
                    width: 100.0,
                    height: 100.0,
                });
                if warm {
                    drop(resolver.get_vector(&url, &request).unwrap());
                }
                let barrier = Barrier::new(workers);
                BYTES.store(0, Ordering::SeqCst);
                CALLS.store(0, Ordering::SeqCst);
                let stop = StopCounting;
                COUNTING.store(true, Ordering::SeqCst);
                let scenes = std::thread::scope(|scope| {
                    let handles = (0..workers)
                        .map(|_| {
                            scope.spawn(|| {
                                barrier.wait();
                                resolver.get_vector(&url, &request).unwrap()
                            })
                        })
                        .collect::<Vec<_>>();
                    handles
                        .into_iter()
                        .map(|thread| thread.join().unwrap())
                        .collect::<Vec<_>>()
                });
                drop(stop);
                let allocated = BYTES.load(Ordering::SeqCst);
                let calls = CALLS.load(Ordering::SeqCst);
                let distinct = scenes
                    .iter()
                    .map(|scene| Arc::as_ptr(scene) as usize)
                    .collect::<HashSet<_>>()
                    .len();
                assert_eq!(fetches.load(Ordering::SeqCst), 1);
                for scene in &scenes {
                    assert_eq!(scene.size, (100.0, 100.0));
                    assert_eq!(scene.as_ref(), scenes[0].as_ref());
                    assert!(scene.node_count() > 1);
                }
                let weak = scenes.iter().map(Arc::downgrade).collect::<Vec<_>>();
                drop(scenes);
                let retained = weak
                    .iter()
                    .filter(|scene| scene.strong_count() > 0)
                    .map(|scene| scene.as_ptr() as usize)
                    .collect::<HashSet<_>>()
                    .len();
                let retained_bytes = weak
                    .iter()
                    .filter_map(|scene| scene.upgrade())
                    .map(|scene| (Arc::as_ptr(&scene) as usize, scene.estimated_heap_bytes()))
                    .collect::<std::collections::HashMap<_, _>>()
                    .values()
                    .sum::<usize>();
                eprintln!(
                    "public scene flight: rectangles={rectangles},workers={workers},warm={warm},allocated={allocated},calls={calls},distinct={distinct},retained={retained},retained_bytes={retained_bytes}"
                );
                if !warm && workers == 1 {
                    serial_bytes = allocated;
                }
                if distinct != 1
                    || retained != 1
                    || (!warm && allocated > serial_bytes + serial_bytes / 4 + 1024 * 1024)
                {
                    failures.push((rectangles, workers, warm, distinct, retained, allocated));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "duplicate scenes or excess allocation: {failures:?}"
    );
}
