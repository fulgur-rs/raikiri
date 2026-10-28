use super::{DomRuntime, StubHost};

#[test]
#[ignore = "manual ownership allocation and initialization probe"]
fn ownership_scaling_probe() {
    let count: usize = std::env::var("RAIKIRI_GC_PROBE_N")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1000);
    let mut elapsed = Vec::new();
    for sample in 0..5 {
        boa_gc::force_collect();
        let (mut host, _, _, body) = StubHost::page();
        for _ in 0..count {
            host.document
                .append_element(Some(body), "div", None::<&str>);
        }
        let before = resident_kib();
        let start = std::time::Instant::now();
        let runtime = DomRuntime::new(host).unwrap();
        elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
        if sample == 0 {
            println!(
                "first_initialization_rss_kib_before={before:?} after={:?}",
                resident_kib()
            );
        }
        std::hint::black_box(&runtime);
        drop(runtime);
    }
    elapsed.sort_by(f64::total_cmp);
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    runtime.evaluate("var probeWeaks = []; (function () { for (var i = 0; i < 128; i++) { var node = document.createElement('div'); node.payload = new Uint8Array(65536); probeWeaks.push(new WeakRef(node.payload)); } })();").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    let live = runtime
        .evaluate("probeWeaks.filter(function (ref) { return ref.deref() !== undefined; }).length")
        .unwrap();
    println!(
        "nodes={count} initialization_median_ms={:.4} retained_64KiB_payloads={live:?}",
        elapsed[2]
    );
}

fn resident_kib() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
