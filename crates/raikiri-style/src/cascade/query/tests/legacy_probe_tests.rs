use super::*;
use std::hint::black_box;
use std::time::Instant;

#[test]
#[ignore = "manual legacy unlimited one-subject allocation/constant overhead probe"]
fn legacy_simple_query_probe() {
    let iterations: usize = std::env::var("SELECTOR_LEGACY_ITERATIONS")
        .ok()
        .map(|n| n.parse().unwrap())
        .unwrap_or(500_000);
    let (dom, div, p) = doc();
    let query = SelectorQuery::parse("p").unwrap();
    let start = Instant::now();
    let mut hits = 0;
    for _ in 0..iterations {
        hits += usize::from(black_box(&query).matches_scoped(black_box(&dom), p, &[div], None));
    }
    assert_eq!(hits, iterations);
    eprintln!(
        "SELECTOR_LEGACY iterations={iterations} elapsed_ns={} hits={hits}",
        start.elapsed().as_nanos()
    );
}
