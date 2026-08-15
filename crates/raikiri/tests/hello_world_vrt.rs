//! hello-world VRT, pinned to WPT bundled fonts。
//!
//! Acceptance criteria: `raikiri::html_to_png_with_fonts(HELLO, font_ctx)`
//! の output が `tests/reference/hello-world/expected/page-0000.png` と
//! byte-identical。`font_ctx` は `target/wpt/fonts` (WPT fetch 済) から
//! `build_wpt_font_ctx` で構築した pinned `FontContext` — cross-machine
//! 決定性のため system font 経路にはフォールバックしない。
//!
//! # 実行方法 (spec §9.2)
//!
//! この test は **`#[ignore]`** — default `cargo test` では走らない
//! (`target/wpt/fonts/` fetch 済を hard requirement とする為、clean checkout
//! では失敗する)。実行手順:
//!
//! ```bash
//! scripts/wpt/fetch.sh                                             # 初回のみ
//! cargo test -p raikiri --test hello_world_vrt -- --ignored
//! ```
//!
//! # `#[ignore]` の trade-off
//!
//! `#[ignore]` は default CI から VRT regression coverage を外す既知の
//! trade-off。この test は cross-machine 決定性の end-to-end 検証を担うが、
//! CI で走らないと regression 検知は developer の local 実行 + `--ignored`
//! に依存する。
//!
//! 選択肢は 3 つあり、user 判断で以下を採用:
//! - **[採用] (B) `#[ignore]` 維持** + CI 連携は
//!   (CI fmt gate 拡張枠) の follow-up として明記
//! - (A) `Ahem.ttf` を repo に direct bundle して `#[ignore]` を外す —
//!   spec の "WPT 経由 fetch" 方針との整合性 tension、将来 reconsider
//! - (C) CI に fetch step を本 branch で追加 — scope creep
//!   (別 follow-up の implementation 前倒し)
//!
//! CI 連携が landing するまでは、`scripts/wpt/fetch.sh` を PR 前 local で
//! 実行 + `-- --ignored` で verify する運用。
//!
//! # Golden 更新
//! `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt -- --ignored`
//! で expected/ を再生成 (raikiri-vrt::reference の UPDATE_GOLDENS_ENV convention)。
//! spec §5.2 に従い Tier 1 (Linux x86_64) は `Tolerance::EXACT` を要求。

use raikiri_dom::build_wpt_font_ctx;
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
