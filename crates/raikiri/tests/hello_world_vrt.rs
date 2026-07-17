//! M1 hello-world VRT (raikiri-spike-m1.14)。
//!
//! spec §M1 acceptance criteria: `raikiri::html_to_png(HELLO)` の output が
//! `tests/reference/hello-world/expected/page-0000.png` と byte-identical。
//!
//! # Golden 更新
//! `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt` で
//! expected/ を再生成 (raikiri-vrt::reference の UPDATE_GOLDENS_ENV convention)。
//! spec §5.2 に従い Tier 1 (Linux x86_64) は `Tolerance::EXACT` を要求。

use raikiri_vrt::reference::{Tolerance, run_and_compare};
use std::path::PathBuf;

#[test]
fn hello_world_renders_pixel_exact() {
    let fixture_dir: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "reference",
        "hello-world",
    ]
    .iter()
    .collect();

    run_and_compare(&fixture_dir, Tolerance::EXACT, |input_bytes| {
        let png = raikiri::html_to_png(input_bytes).expect("html_to_png must succeed");
        vec![png]
    });
}
