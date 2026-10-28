use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

#[test]
#[ignore = "manual release-mode Window named lookup measurement"]
fn window_named_miss_scaling_probe() {
    use std::time::Instant;
    let source = "(() => { let found = false; for (let i = 0; i < 1000; ++i) { found ||= ('__perf996_missing__' in window); } return found; })()";
    for n in [1_000, 2_000, 4_000, 8_000] {
        let mut samples = Vec::new();
        for _ in 0..5 {
            let (mut host, _, _, body) = StubHost::page();
            for _ in 0..n {
                host.document.append_element(Some(body), "div", None::<&str>);
            }
            let mut runtime = DomRuntime::new(host).unwrap();
            let start = Instant::now();
            let result = runtime.evaluate(source).unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            assert!(!result.to_boolean());
        }
        samples.sort_by(f64::total_cmp);
        eprintln!("window named miss n={n} queries=1000 median_ms={:.6} samples={samples:?}", samples[2]);
    }
}
