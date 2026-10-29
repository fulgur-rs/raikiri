//! Generated-image piece lookup with decode and layout outside paint timing.
//! Run cargo bench -p raikiri-paint --features generated-image-diagnostics
//! --bench content_image_pieces. Counters run separately from timing.

// cov:ignore: standalone benchmark is not built or instrumented by workspace coverage.
mod accounting {
    use anyrender::{Scene, recording::RenderCommand};
    use raikiri_dom::fonts::{BundledFace, build_bundled_font_collection};
    use raikiri_dom::{Document, PositionedLines, layout_single_page};
    use raikiri_paint::{generated_image_diagnostics, paint_single_page};
    use raikiri_style::{CascadeResult, build_rule_tree, cascade};
    use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed},
    };
    use std::time::Instant;

    static TRACK_HEAP: AtomicBool = AtomicBool::new(false);
    static COUNT_REQUESTS: AtomicBool = AtomicBool::new(false);
    static PEAK: AtomicIsize = AtomicIsize::new(0);
    static LIVE: AtomicIsize = AtomicIsize::new(0);
    static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0);

    struct Counting;

    fn allocation(size: usize) {
        if TRACK_HEAP.load(Relaxed) {
            let live = LIVE.fetch_add(size as isize, Relaxed) + size as isize;
            PEAK.fetch_max(live, Relaxed);
        }
        if COUNT_REQUESTS.load(Relaxed) {
            ALLOC_BYTES.fetch_add(size as u64, Relaxed);
            ALLOC_CALLS.fetch_add(1, Relaxed);
        }
    }

    // SAFETY: every method forwards System's arguments and results unchanged;
    // the side counters do not allocate or access the allocation's contents.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's allocation contract.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                allocation(layout.size());
            }
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's allocation contract.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                allocation(layout.size());
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: ptr and layout satisfy System's deallocation contract.
            unsafe { System.dealloc(ptr, layout) };
            if TRACK_HEAP.load(Relaxed) {
                LIVE.fetch_sub(layout.size() as isize, Relaxed);
            }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's reallocation contract.
            let new = unsafe { System.realloc(ptr, layout, size) };
            if !new.is_null() {
                if TRACK_HEAP.load(Relaxed) {
                    let live = LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed)
                        + size as isize
                        - layout.size() as isize;
                    PEAK.fetch_max(live, Relaxed);
                }
                if COUNT_REQUESTS.load(Relaxed) {
                    REALLOC_BYTES.fetch_add(size as u64, Relaxed);
                    REALLOC_CALLS.fetch_add(1, Relaxed);
                }
            }
            new
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    struct Pixels(Arc<DecodedImage>);
    impl ImagePixelSource for Pixels {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            Some(self.0.clone())
        }
    }

    struct Prepared {
        document: Document,
        cascade: CascadeResult,
        page: PageBox,
        pieces: usize,
        images: usize,
    }

    impl Prepared {
        fn new(count: usize, images: bool, install: &impl Fn(&mut Document)) -> Self {
            let mut document = Document::new();
            install(&mut document);
            let html = document.append_element(Some(0), "html", Some("display:block"));
            let body = document.append_element(Some(html), "body", Some("display:block;margin:0"));
            let root = document.append_element(Some(body), "div", Some(
                "display:block;white-space:nowrap;font-family:Ahem;font-size:10px;line-height:10px"
            ));
            let style = document.append_element(Some(html), "style", None::<&str>);
            if images {
                document.append_text(
                    style,
                    "span::before{content:url(https://images.test/a.png)}",
                );
            }
            for _ in 0..count {
                let span = document.append_element(Some(root), "span", None::<&str>);
                document.append_text(span, "a");
            }
            document.mark_in_document_flags();
            let cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
            let pixels = Pixels(Arc::new(DecodedImage::new(1, 1, vec![0, 0, 255, 255])));
            document.prepare_list_marker_images(&cascade, &pixels, None);
            let mut page = PageBox::A4;
            page.width = (count * 12 + 100) as f32;
            layout_single_page(&mut document, &cascade, page).unwrap();
            let positioned = PositionedLines::new(&document, &cascade, root, None).unwrap();
            let lines: Vec<_> = positioned.lines().collect();
            assert_eq!(lines.len(), 1);
            let placed = lines[0].content_images.len();
            assert_eq!(placed, if images { count } else { 0 });
            let pieces = document
                .ifc_layout_node(root)
                .unwrap()
                .ifc_inline_boxes()
                .unwrap()
                .len();
            assert!(pieces >= count);
            Self {
                document,
                cascade,
                page,
                pieces,
                images: placed,
            }
        }

        fn paint(&self) -> Scene {
            let mut scene = Scene::new();
            paint_single_page(&mut scene, &self.document, &self.cascade, self.page).unwrap();
            scene
        }

        fn verify(&self, scene: &Scene) {
            let images = scene.commands.iter().filter(|command| matches!(command,
                RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_))
            )).count();
            assert_eq!(images, self.images);
        }
    }

    fn measure(count: usize, images: bool, install: &impl Fn(&mut Document)) {
        let prepared = Prepared::new(count, images, install);
        prepared.verify(&prepared.paint());
        LIVE.store(0, Relaxed);
        PEAK.store(0, Relaxed);
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        COUNT_REQUESTS.store(true, Relaxed);
        generated_image_diagnostics::begin();
        let scene = prepared.paint();
        let work = generated_image_diagnostics::finish();
        COUNT_REQUESTS.store(false, Relaxed);
        let retained = LIVE.load(Relaxed);
        let peak = PEAK.load(Relaxed);
        prepared.verify(&scene);
        drop(scene);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(after_drop, 0);
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let scene = prepared.paint();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            prepared.verify(&scene);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "spans={count} images={} pieces={} predicate_visits={} index_visits={} image_lookups={} alloc_bytes={} realloc_bytes={} allocs={} reallocs={} peak_added_heap={peak} scene_retained_heap={retained} after_drop={after_drop} paint_ms={:.6}",
            prepared.images,
            prepared.pieces,
            work.predicate_visits,
            work.index_visits,
            work.image_lookups,
            ALLOC_BYTES.load(Relaxed),
            REALLOC_BYTES.load(Relaxed),
            ALLOC_CALLS.load(Relaxed),
            REALLOC_CALLS.load(Relaxed),
            times[3]
        );
    }

    pub fn run() {
        let fonts = build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf")
                    .to_vec(),
            }],
            false,
        )
        .unwrap();
        let install = |document: &mut Document| document.set_font_collection(fonts.clone());
        let counts: Vec<usize> = std::env::var("RAIKIRI_IMAGE_PIECES")
            .ok()
            .map(|s| s.split(',').map(|n| n.parse().unwrap()).collect())
            .unwrap_or_else(|| vec![1, 100, 500, 1000, 2000]);
        for count in counts {
            measure(count, true, &install);
            measure(count, false, &install);
        }
    }
}

// cov:ignore: standalone benchmark entry point is not instrumented by workspace coverage.
fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        accounting::run();
    }
}
