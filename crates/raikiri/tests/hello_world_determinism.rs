//! M1 determinism test (raikiri-spike-m1.13)。
//!
//! spec §12.3 / §12.6 acceptance criteria: `raikiri::html_to_png` を同一 process
//! 内で 10 回連続実行し、全 output が byte-identical であることを verify。
//! 同一 input が同一 output に決定論的に mapping されることを保証する M1 acceptance
//! criteria の一部。
//!
//! spec §12.8 の他次元 (Rayon threads, Process, Arch/OS, Fonts, Dep upgrade) は
//! M8 の cross-thread-cross-arch-cross-os-determinism-tests task で full matrix
//! 化する設計。この test は same-process の base-case のみ担う。

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
