//! M1 hello-world VRT (raikiri-spike-m1.14), pinned to WPT bundled fonts
//! (raikiri-spike-e93)。
//!
//! spec §M1 acceptance criteria: `raikiri::html_to_png_with_fonts(HELLO, font_ctx)`
//! の output が `tests/reference/hello-world/expected/page-0000.png` と
//! byte-identical。`font_ctx` は `target/wpt/fonts` (WPT fetch 済) から
//! `build_wpt_font_ctx` で構築した pinned `FontContext` — cross-machine
//! 決定性のため system font 経路にはフォールバックしない。
//!
//! # Golden 更新
//! `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt` で
//! expected/ を再生成 (raikiri-vrt::reference の UPDATE_GOLDENS_ENV convention)。
//! spec §5.2 に従い Tier 1 (Linux x86_64) は `Tolerance::EXACT` を要求。

use raikiri_dom::build_wpt_font_ctx;
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

    // WPT bundled fonts の fetch 済を前提とする (cross-machine 決定性のため
    // system font 経路にはフォールバックしない)。Fetch 未実行時は明示 panic。
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

    let font_ctx = build_wpt_font_ctx(&fonts_dir).expect("build_wpt_font_ctx must succeed");

    run_and_compare(&fixture_dir, Tolerance::EXACT, |input_bytes| {
        let png = raikiri::html_to_png_with_fonts(input_bytes, font_ctx)
            .expect("html_to_png_with_fonts must succeed");
        vec![png]
    });
}
