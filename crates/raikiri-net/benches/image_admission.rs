//! Image cache misses with an injected wait, measured through the public API.
//! Run cargo bench -p raikiri-net --bench image_admission.
//! This is not a production HTTP or CPU decode throughput measurement.

// cov:ignore: standalone benchmark is not built or instrumented by workspace coverage.
mod probe {
    use raikiri_net::ImageResolver;
    use raikiri_traits::{
        FetchOutcome, FetchedResource, ImagePixelSource, NetworkError, NetworkProvider,
        ReplacedResolver, Request, ResolverRequest,
    };
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering::SeqCst},
    };
    use std::time::{Duration, Instant};

    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240,
        159, 17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ];

    #[derive(Default)]
    struct Stats {
        active: AtomicUsize,
        peak: AtomicUsize,
        calls: AtomicUsize,
    }

    struct Active<'a>(&'a Stats);
    impl Drop for Active<'_> {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, SeqCst);
        }
    }

    struct Provider(Arc<Stats>);
    impl NetworkProvider for Provider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let active = self.0.active.fetch_add(1, SeqCst) + 1;
            let _active = Active(&self.0);
            self.0.peak.fetch_max(active, SeqCst);
            self.0.calls.fetch_add(1, SeqCst);
            std::thread::sleep(Duration::from_millis(40));
            Ok(FetchOutcome::Body(
                FetchedResource::new(PNG.to_vec().into(), request.url)
                    .content_type(Some("image/png".into())),
            ))
        }
    }

    fn sample(mode: &str, workers: usize) -> (f64, usize, usize) {
        let stats = Arc::new(Stats::default());
        let start = Arc::new(Barrier::new(workers + 1));
        let shared = Arc::new(ImageResolver::new(Provider(stats.clone())));
        let mut threads = Vec::new();
        for index in 0..workers {
            let resolver = if mode == "independent" {
                Arc::new(ImageResolver::new(Provider(stats.clone())))
            } else {
                shared.clone()
            };
            let start = start.clone();
            let key = if mode == "duplicate" { 0 } else { index };
            threads.push(std::thread::spawn(move || {
                let url = format!("https://images.test/{key}.png").parse().unwrap();
                start.wait();
                let got = resolver.resolve(ResolverRequest::new(&url)).unwrap();
                assert_eq!((got.intrinsic.width, got.intrinsic.height), (2.0, 1.0));
                let pixels = resolver.get_decoded(&url).unwrap();
                assert_eq!((pixels.width, pixels.height, pixels.rgba.len()), (2, 1, 8));
                let cached = resolver.resolve(ResolverRequest::new(&url)).unwrap();
                assert_eq!(
                    (cached.intrinsic.width, cached.intrinsic.height),
                    (2.0, 1.0)
                );
                assert!(Arc::ptr_eq(&pixels, &resolver.get_decoded(&url).unwrap()));
            }));
        }
        let clock = Instant::now();
        start.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        let elapsed = clock.elapsed().as_secs_f64() * 1000.0;
        let calls = stats.calls.load(SeqCst);
        let peak = stats.peak.load(SeqCst);
        assert_eq!(calls, if mode == "duplicate" { 1 } else { workers });
        assert_eq!(stats.active.load(SeqCst), 0);
        assert!(peak > 0 && peak <= workers);
        (elapsed, peak, calls)
    }

    pub fn run() {
        for mode in ["shared", "independent", "duplicate"] {
            sample(mode, 1);
        }
        for workers in [1, 2, 4, 8] {
            for mode in ["shared", "independent", "duplicate"] {
                let samples: Vec<_> = (0..3).map(|_| sample(mode, workers)).collect();
                let mut times: Vec<_> = samples.iter().map(|sample| sample.0).collect();
                times.sort_by(f64::total_cmp);
                let peaks: Vec<_> = samples.iter().map(|sample| sample.1).collect();
                let calls: Vec<_> = samples.iter().map(|sample| sample.2).collect();
                println!(
                    "mode={mode} workers={workers} elapsed_ms={:.6} samples_ms={:.6},{:.6},{:.6} peaks={},{},{} calls={},{},{}",
                    times[1],
                    samples[0].0,
                    samples[1].0,
                    samples[2].0,
                    peaks[0],
                    peaks[1],
                    peaks[2],
                    calls[0],
                    calls[1],
                    calls[2]
                );
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point is not instrumented by workspace coverage.
fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        probe::run();
    }
}
