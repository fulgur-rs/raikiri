//! Determinism test.
//!
//! Spec §12.3 / §12.6 acceptance criteria: run `raikiri::html_to_png` ten times
//! in the same process and verify that all outputs are byte-identical.
//! This is part of the acceptance criteria guaranteeing a deterministic mapping
//! from identical input to identical output.
//!
//! Other dimensions in spec §12.8 (Rayon threads, process, arch/OS, fonts, dep upgrades)
//! are planned as a full matrix in the future cross-thread-cross-arch-cross-os-determinism-tests
//! task. This test covers only the same-process base case.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

const ITERATIONS: usize = 10;

#[test]
fn hello_world_is_byte_identical_across_10_runs() {
    let input_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "reference",
        "hello-world",
        "input.html",
    ]
    .iter()
    .collect();
    let input = fs::read(&input_path).expect("hello-world input.html must exist");

    let iter0 = raikiri::html_to_png(Cursor::new(&input)).expect("iter 0 html_to_png must succeed");

    for iter in 1..ITERATIONS {
        let iter_n = raikiri::html_to_png(Cursor::new(&input))
            .unwrap_or_else(|e| panic!("iter {iter} html_to_png must succeed: {e:?}"));
        assert_pngs_byte_identical(&iter0, &iter_n, iter);
    }
}

fn assert_pngs_byte_identical(base: &[u8], candidate: &[u8], iter: usize) {
    if base.len() != candidate.len() {
        panic!(
            "iter {iter} PNG length differs from iter 0: expected {} bytes, got {} bytes",
            base.len(),
            candidate.len(),
        );
    }
    if let Some(offset) = base.iter().zip(candidate).position(|(a, b)| a != b) {
        panic!(
            "iter {iter} PNG differs from iter 0: len={}, first differing byte at offset {} \
             (expected 0x{:02x}, got 0x{:02x})",
            base.len(),
            offset,
            base[offset],
            candidate[offset],
        );
    }
}
