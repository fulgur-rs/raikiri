//! hello-world VRT pinned to WPT bundled fonts.
//!
//! Acceptance criteria: `raikiri::html_to_png_with_render_fonts(HELLO, fonts)`
//! produces output byte-identical to `tests/reference/hello-world/expected/page-0000.png`.
//! `fonts` is a font set of `Ahem.ttf` from `target/wpt/fonts` (after WPT fetch)
//! built by `FontCollectionBuilder`, whose generic families all map to Ahem.
//! For cross-machine determinism, it does not fall back to system fonts.
//!
//! # How to run (spec §9.2)
//!
//! This test is **`#[ignore]`**: default `cargo test` does not run it
//! (it requires fetched `target/wpt/fonts/` and would fail on a clean checkout).
//! Run it as follows:
//!
//! ```bash
//! scripts/wpt/fetch.sh                                             # first time only
//! cargo test -p raikiri --test hello_world_vrt -- --ignored
//! ```
//!
//! # `#[ignore]` trade-off
//!
//! `#[ignore]` removes VRT regression coverage from default CI, a known
//! trade-off. This test verifies cross-machine determinism end to end, but until
//! it runs in CI, detecting regressions depends on developers running it locally
//! with `--ignored`.
//!
//! Three options were considered; the user chose:
//! - **[Chosen] (B) Keep `#[ignore]`** and record CI integration as
//!   a follow-up (in the CI fmt gate extension scope).
//! - (A) Bundle `Ahem.ttf` directly in the repository and remove `#[ignore]`:
//!   conflicts with the spec's "fetch via WPT" policy; reconsider later.
//! - (C) Add a CI fetch step on this branch: scope creep,
//!   prematurely implementing a separate follow-up.
//!
//! Until CI integration lands, run `scripts/wpt/fetch.sh` locally before a PR
//! and verify with `-- --ignored`.
//!
//! # Updating goldens
//! `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt -- --ignored`
//! regenerates expected/ (following raikiri-vrt::reference's UPDATE_GOLDENS_ENV convention).
//! Spec §5.2 requires `Tolerance::EXACT` for Tier 1 (Linux x86_64).

use raikiri_vrt::reference::{Tolerance, run_and_compare};
use std::path::PathBuf;

#[test]
#[ignore = "requires scripts/wpt/fetch.sh; run with --ignored"]
fn hello_world_renders_pixel_exact() {
    let fixture_dir: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "reference",
        "hello-world",
    ]
    .iter()
    .collect();

    // WPT bundled fonts must have been fetched (no system-font fallback, to preserve
    // cross-machine determinism). Panic explicitly if they have not been fetched.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fonts_dir: PathBuf = [manifest_dir, "..", "..", "target", "wpt", "fonts"]
        .iter()
        .collect();

    assert!(
        fonts_dir.exists(),
        "target/wpt/fonts not found at {} — run scripts/wpt/fetch.sh first",
        fonts_dir.display()
    );
    assert!(
        fonts_dir.join("Ahem.ttf").exists(),
        "Ahem.ttf missing under {} — WPT pin (scripts/wpt/pinned_sha.txt) may need bump",
        fonts_dir.display()
    );

    let ahem = std::fs::read(fonts_dir.join("Ahem.ttf")).expect("Ahem.ttf must be readable");
    let fonts = raikiri::FontCollectionBuilder::new()
        .font_bytes("Ahem", ahem)
        .build()
        .expect("the Ahem font set must build");

    run_and_compare(&fixture_dir, Tolerance::EXACT, |input_bytes| {
        let png = raikiri::html_to_png_with_render_fonts(input_bytes, fonts.clone())
            .expect("html_to_png_with_render_fonts must succeed");
        vec![png]
    });
}
