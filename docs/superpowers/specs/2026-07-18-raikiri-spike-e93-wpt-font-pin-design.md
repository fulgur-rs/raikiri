---
title: raikiri-spike-e93 — parley::FontContext を WPT bundled font に pin (VRT cross-machine 決定性) — 設計
status: Draft
date: 2026-07-18
author: Mitsuru Hayasaka (@mitsuru)
issue: raikiri-spike-e93
related:
  - "beads: raikiri-spike-e93 (P2, bug)"
  - "roborev job 253 (codex agent) m1.14 hello-world VRT Medium finding"
  - "m1.13 hello_world_determinism.rs (within-machine 10-iter byte-identical, m1.13)"
  - "m1.14 hello-world VRT fixture: crates/raikiri/tests/reference/hello-world/ (input.html + expected/page-0000.png)"
  - "fulgur `scripts/wpt/` (shallow sparse checkout pattern, 参考)"
  - "fulgur `crates/fulgur-wpt/src/fonts.rs` (walker + register, 参考)"
  - "blitz `packages/blitz-dom/src/lib.rs::build_single_font_ctx` (system_fonts: false + generic alias, 参考)"
  - "spec §M1 'ASCII (Latin) 単一 script + 単一 font に限定'"
  - "raikiri-spike-2sb (@font-face / WOFF, M4+)"
  - "raikiri-spike-rcf (CI fmt gate, CI 統合の後追い)"
---

# raikiri-spike-e93 — parley::FontContext を WPT bundled font に pin (VRT cross-machine 決定性) — 設計

## 1. 背景

roborev job 253 (m1.14 hello-world VRT review) の Medium finding として、
`raikiri_dom::layout::preshape_text` が `parley::FontContext::new()` を default で
構築し、fontique の system font resolver (fontconfig / CoreText / DirectWrite) を
経由することが指摘された。

- Production call site: `crates/raikiri-dom/src/layout.rs:186` (`layout_single_page` 内)
- Test call site: `crates/raikiri-dom/src/layout.rs:292`, `:337`,
  `crates/raikiri-dom/src/lib.rs:573`
- Perf test (無関係): `crates/raikiri-dom/src/layout.rs:512`

hello-world VRT (`<p style="color:red">Hi</p>`) は inline `font-family` を持たず、
UA CSS cascade 経由で `[Atom::from("serif")]`
(`crates/raikiri-style/src/computed.rs:35`) が渡る。parley の `FontFamily::from("serif")`
は generic family として fontique に丸投げされ、**host の fontconfig 設定 /
font package version が違うと同じ HTML から異なる glyph 位置と bitmap が出る**。

結果、m1.14 hello-world VRT (`Tolerance::EXACT`, golden md5=203c44848d3582f1cfa121d587dad0ed
at 2026-07-17 local) の golden PNG が CI runner image update や別マシンで break する。

現状の局所回避:
- m1.14 の golden は現ローカル環境で bootstrap 済
- 同一マシン 3 連続 md5 一致 (m1.14 AC #7 within-machine 決定性成立)
- **cross-machine 決定性は未担保**

## 2. Design Goals / Non-Goals

### Goals

- **VRT 経路の cross-machine 決定性** — 同 HTML 入力 → 同 PNG bytes を任意
  マシン間で保証する (M1 scope の Latin ASCII 単一 font 前提下)
- **fontique の system font resolver の完全 bypass** — VRT 経路で
  `system_fonts: false` を強制し、drift の構造的原因を根絶
- **既存 `raikiri::html_to_png` API の互換維持** — production runtime は
  system font 経路のまま、pub API 破壊なし
- **fulgur / blitz と実装 shape を揃える** — `scripts/wpt/` と `build_wpt_font_ctx`
  は fulgur `crates/fulgur-wpt/src/fonts.rs` + blitz `build_single_font_ctx` の
  合成 pattern
- **dep cycle 回避** — `raikiri-dom → raikiri-wpt` の逆流を作らない

### Non-Goals

- Production runtime での automatic font pin (VRT test 経路のみ pin)
- WOFF / WOFF2 decode (M4+ / [[raikiri-spike-2sb]])
- Multi-font fallback (M3 text-multilingual)
- BiDi / CJK / Emoji font (M3)
- `FontInfoOverride` 経由の CSS ⇔ TTF metadata rewrite (@font-face M4+)
- Multi-container CI matrix / docker cross-verify ([[raikiri-spike-rcf]] 枠)
- WPT test 用 subset の拡張 (本 issue の scope 外、M2+ で別 issue)
- Struct-based `Options` / builder pattern の `html_to_png` API 拡張
  ([[raikiri-spike-9cy]] migration_hint 議論と合流)

## 3. Architecture Overview

### 3.1 Component 分け

```
raikiri-spike/
├── scripts/wpt/           ← fulgur から copy-adapt
│   ├── fetch.sh
│   ├── subset.txt          (M1 scope: "fonts")
│   ├── pinned_sha.txt
│   └── README.md
│
├── target/wpt/fonts/       ← gitignore、fetch 出力先
│
└── crates/
    ├── raikiri-dom/
    │   └── src/fonts.rs    ← 新規: build_wpt_font_ctx + walker + generic alias
    ├── raikiri/
    │   └── src/lib.rs      ← 追加: html_to_png_with_fonts()
    └── raikiri/tests/
        └── hello_world_vrt.rs  ← 更新: build_wpt_font_ctx + html_to_png_with_fonts
```

### 3.2 Dep 方向 (単一)

```
scripts/wpt/               (workspace root, dep 無)
    ↓ (fetch 出力)
target/wpt/fonts/           (gitignore)
    ↑ (Path 参照)
raikiri-dom::fonts          (parley::fontique の pub API のみ使用)
    ↑
raikiri::html_to_png_with_fonts
    ↑
raikiri/tests/hello_world_vrt.rs
```

- raikiri-dom は raikiri-wpt を knowledge しない (cycle 回避)
- raikiri-wpt は本 issue で変更なし (unchanged)
- test 層が唯一 fetch 実行 (dev prerequisite) を要求

### 3.3 Production runtime の不変性

`raikiri::html_to_png(input: impl Read)` (既存 pub API) は system font 経路のまま
維持。M1.14 で確定した signature は unchanged。VRT 経路のみ新規追加 API
`html_to_png_with_fonts` を通す。

## 4. WPT Fetch Pipeline

### 4.1 File layout

```
raikiri-spike/scripts/wpt/
├── fetch.sh           ← shallow clone + sparse-checkout → target/wpt/
├── subset.txt         ← "fonts" のみ (M1)
├── pinned_sha.txt     ← WPT upstream SHA pin
└── README.md          ← 手順書 + pin 更新プロセス
```

### 4.2 fetch.sh (fulgur から copy-adapt)

- `git init` + `--filter=blob:none` partial clone + `sparse.checkout=true` core
- `WPT_REMOTE_URL` env で mirror URL override
- 冪等: 再実行で pinned SHA まで advance
- 出力先 `target/wpt/`、`.git/` は残す (再 fetch のため)
- 差分は fulgur 版から `REPO_ROOT` の指す先だけ

### 4.3 subset.txt

M1 scope の最小 line only:

```
fonts
```

これで WPT `fonts/` dir 全体 (Ahem, Lato, CSSTest, RobotoMono など) が
`target/wpt/fonts/` に checkout される。M2+ で `css/css-page` などの test subset
追加は本 issue の scope 外 (別 issue spin out、[[raikiri-spike-x7a]] の M3 kickoff
タイミングと自然にリンク)。

### 4.4 pinned_sha.txt

**方針: fulgur pin を default、必要に応じ raikiri 専有 bump**。

- 初回は fulgur の pin (`97ea26e26a2aac3eec7e770650b25e7049ed4a4e`, 2026-04-21) を採用
- README に "raikiri 専有 bug で fresh pin 必要なら fulgur にとらわれず bump 可、
  ただし drift 追跡は raikiri 側で" と明記
- pin bump は PR 経由、`scripts/wpt/fetch.sh && cargo test -p raikiri --test hello_world_vrt`
  で verify

### 4.5 Fetch trigger

- **手動**: developer は初回 checkout 後に `scripts/wpt/fetch.sh` を実行
- **build.rs での automatic fetch は採用しない** (network fetch in build は
  air-gapped CI と offline dev で brittle)
- VRT test 側で `target/wpt/fonts/` の存在チェック、なければ **panic with 明示的
  error message** ("Run scripts/wpt/fetch.sh first")

### 4.6 CI 統合

CI cache key に `pinned_sha.txt` の hash を含めて subsequent job で reuse。
具体 CI 実装は本 issue で触らない ([[raikiri-spike-rcf]] の CI fmt gate と同じ枠で後追い)。

### 4.7 gitignore

workspace root で `target/` は既に ignore 済 (確認済)。追加 pattern 不要。

## 5. raikiri-dom Font Module

### 5.1 File location

`crates/raikiri-dom/src/fonts.rs` (新規)

### 5.2 Public API

```rust
// crates/raikiri-dom/src/fonts.rs
use parley::FontContext;
use std::path::Path;

/// WPT-bundled font dir を全 register し、generic family alias を pin した
/// FontContext を構築する。system font resolver は完全 disable。
///
/// # 決定性
/// - dir 内 entries は path 順 sort 後 register (fallback 順序の決定性)
/// - system_fonts: false で fontique の platform resolver bypass
/// - generic family (Serif/SansSerif/Monospace/SystemUi/Cursive/Fantasy)
///   すべてに register 済 family_ids を append
/// - `PREFERRED_FIRST` に matching する path (Lato-* 4 個) を先頭に move
///
/// # M1 scope
/// - `.ttf` / `.otf` のみ受付 (WOFF/WOFF2 は M4+ / [[raikiri-spike-2sb]])
/// - dir missing 時は `Err(FontError::DirNotFound)` 返却 (fulgur pattern と異なり
///   silent empty を避け、pit-of-success を優先)
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError>;

/// M1 scope: `.ttf` / `.otf` のみ受付。
#[derive(Debug)]
pub enum FontError {
    /// `fonts_dir` が存在しない
    DirNotFound(std::path::PathBuf),
    /// `fonts_dir` は存在するが `.ttf` / `.otf` が 1 個も無い
    EmptyDir(std::path::PathBuf),
    /// io error during walking
    Io { path: std::path::PathBuf, source: std::io::Error },
}

// Display + std::error::Error impl は手書き (thiserror 依存追加なし、既存
// LayoutError / RenderError と shape 揃える)
```

### 5.3 内部 walker

```rust
const PREFERRED_FIRST: &[&str] = &[
    "Lato-Regular.ttf",
    "Lato-Bold.ttf",
    "Lato-Italic.ttf",
    "Lato-BoldItalic.ttf",
];

// 1. dir を recursive walk、.ttf/.otf を collect
// 2. all.sort() で path 順
// 3. PREFERRED_FIRST にある file_name を先頭に partition
// 4. preferred は PREFERRED_FIRST 配列の index 順に再ソート
// 5. rest はそのまま path sort 順
// 6. 結果 iter で Collection::register_fonts + family_ids collect
// 7. append_generic_families で generic 6 種に family_ids を append
```

- Extension filter: `.ttf` / `.otf` (小文字 normalize)
- 失敗した font は `eprintln!("[raikiri-dom::fonts] warn: skipping {path}: {err}")`
  で skip (parse-invalid ttf を許容、fatal にしない)
- Missing PREFERRED_FIRST エントリ (Lato がない場合) は silent skip (VRT test 側で
  存在 assertion して safety net)

### 5.4 Generic alias remap (blitz pattern)

```rust
use parley::fontique::GenericFamily;

for generic in [
    GenericFamily::Serif,
    GenericFamily::SansSerif,
    GenericFamily::Monospace,
    GenericFamily::SystemUi,
    GenericFamily::Cursive,
    GenericFamily::Fantasy,
] {
    ctx.collection.append_generic_families(generic, family_ids.iter().copied());
}
```

- `Emoji` / `UiSerif` / `UiSansSerif` / `UiMonospace` / `UiRounded` / `Math` は M1
  scope 外 (WPT fonts に該当 asset なし、cascade で使われる可能性なし)
- Register 順序 = fallback 順序 (parley/fontique 仕様)、walker sort の結果
  Lato-Regular が先頭 → cascade `"serif"` は Lato-Regular に解決

### 5.5 依存関係

- `parley` (既存 workspace dep) の `FontContext`, `fontique::{Blob, Collection,
  CollectionOptions, GenericFamily, SourceCache}` のみ使用
- `std::path`, `std::fs`, `std::io`, `std::sync::Arc` のみ (external crate 追加なし)
- `thiserror` / `log` / `tracing` 依存追加なし (raikiri 慣習に準拠)

## 6. raikiri `html_to_png_with_fonts`

### 6.1 New public API

```rust
// crates/raikiri/src/lib.rs
use parley::FontContext;

/// html_to_png の font-aware 版。cross-machine 決定性が必要な VRT test で
/// 使う。渡された FontContext がそのまま layout に使われ、system font
/// resolver は完全 bypass される。
///
/// # M1 scope
/// - VRT test 向け。production runtime は既存 [`html_to_png`] を使う
/// - M4+ で @font-face が入るタイミングで production consumer からの利用も検討
pub fn html_to_png_with_fonts(
    input: impl std::io::Read,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError>;
```

### 6.2 既存 `html_to_png` との関係

- 既存: `pub fn html_to_png(input: impl Read) -> Result<Vec<u8>, RenderError>` —
  signature unchanged
- 実装は **共通化 (DRY)**: `html_to_png(input)` は internally
  `html_to_png_with_fonts(input, FontContext::new())` に delegate、layout logic
  を 1 箇所に集約 (production 経路と VRT 経路の drift 防止)

### 6.3 `layout_single_page` の signature 変更 (β)

`crates/raikiri-dom/src/layout.rs:172` の `layout_single_page` の signature を
`FontContext` 追加受取に変更:

```rust
// before
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError>;

// after
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    font_ctx: FontContext,
) -> Result<(), LayoutError>;
```

内部 (現 layout.rs:186) の `FontContext::new()` は削除、引数の `font_ctx` を使用。

### 6.4 caller update (12 箇所)

`layout_single_page` の caller 内訳:

**production 経路 (1 箇所)** — 本 spec の中核変更:
- `crates/raikiri/src/html_to_png.rs:47`
- `layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box)` →
  `layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box, font_ctx)`
- ここに §6.2 delegate 経路の font_ctx が流れる

**test 経路 (11 箇所, mechanical rename)**:
- `crates/raikiri-paint/src/lib.rs`: 5 箇所 (line 69, 152, 199, 307, 379)
- `crates/raikiri-dom/src/layout.rs` `#[cfg(test)]`: 6 箇所 (line 398, 425, 441,
  448, 464, 479)
- `layout_single_page(&mut doc, &cr, PageBox::A4)` → `layout_single_page(&mut doc,
  &cr, PageBox::A4, FontContext::new())`
- 全て determinism 不要な paint/layout test で `FontContext::new()` (system font 経路) で OK

### 6.5 preshape_text 直呼び test は影響なし

`crates/raikiri-dom/src/layout.rs` 内 `#[cfg(test)]` module の `:280` (line 280
の test `preshape_text_populates_text_layout`) と `:325` (line 325 の test
`preshape_text_respects_computed_font_size`) は `preshape_text` を直呼びしていて
既に `FontContext::new()` を明示的に生成している。`preshape_text` の signature
(`fonts: &mut FontContext` 注入型) は変更しないので、これらの test は
**そのまま unchanged**。

`crates/raikiri-dom/src/lib.rs:573` 内 test も `layout_cx.ranged_builder` の
直呼びで、`layout_single_page` は経由しない。unchanged。

`crates/raikiri-dom/src/layout.rs:512` の `font_context_new_cost_is_reasonable`
は `FontContext::new()` 単体の speed test、determinism と無関係、unchanged。

### 6.6 Error 型

`RenderError` に variant 追加なし。`html_to_png_with_fonts` は既存 `html_to_png`
と同 error 型を返す。

## 7. UA CSS / Cascade Behavior

### 7.1 UA CSS default は unchanged

`crates/raikiri-style/src/computed.rs:35` の initial value
`[Atom::from("serif")]` は変更しない。cascade は `"serif"` を渡し続ける。

### 7.2 Cascade `"serif"` の解決経路

1. hello-world VRT input `<p style="color:red">Hi</p>` は inline `font-family` なし
2. raikiri-style cascade で `font_family = [Atom::from("serif")]` (UA initial から inherit)
3. `preshape_text` 内で `FontFamily::from("serif")` を生成し `layout_cx.ranged_builder`
   に渡す
4. parley/fontique が **generic family "serif" を解決** — 通常は system resolver 経由
5. しかし本 spec の `build_wpt_font_ctx` で:
   - `system_fonts: false` により system resolver は空
   - `append_generic_families(GenericFamily::Serif, [Lato_id, Ahem_id, ...])`
     により Serif generic に Lato-Regular (walker sort の先頭) が解決
6. **cascade `"serif"` cascade は Lato-Regular に解決** → hello-world VRT "Hi"
   は real text visual で描画される

### 7.3 hello-world VRT の visual

- Before (system font, host-dependent): platform serif の "Hi"
- After (WPT Lato via generic alias): Lato-Regular の "Hi"

`crates/raikiri/tests/reference/hello-world/README.md` に "Lato-Regular 経由の
visual" を追記、golden 再生成手順を残す。

## 8. Testing Strategy

### 8.1 `raikiri-dom/src/fonts.rs` unit tests

- `missing_dir_returns_err` — 不在 path で `Err(FontError::DirNotFound)`
- `empty_dir_returns_empty_dir_err` — 空 dir で `Err(FontError::EmptyDir)`
- `loads_ttf_files` — fake ttf 2 個 → family_ids 2 entries
- `ignores_non_font_extensions` — `.md` / `.txt` は skip
- `recurses_into_subdirs` — subdir 下の ttf も pick up
- `sort_order_is_deterministic` — 同 dir を 2 回 build して register 順一致
- `preferred_first_orders_lato_before_ahem` — Ahem.ttf と Lato-Regular.ttf 両方
  置いた dir で family_ids[0] = Lato の family_id
- `generic_serif_resolves_to_registered_family` — `build_wpt_font_ctx` 後の ctx
  で `FontFamily::from("serif")` resolve が register 済 family に成功 (parley の
  resolution query API 経由、詳細は plan phase で確定)

### 8.2 layout signature 追従

`layout_single_page` signature 変更 (§6.3) に伴う caller update (§6.4):

- production: `crates/raikiri/src/html_to_png.rs:47` 1 箇所 (delegate 経路の
  font_ctx を pass)
- test: `crates/raikiri-paint/src/lib.rs` 5 箇所 + `crates/raikiri-dom/src/layout.rs`
  内 6 箇所 = 計 11 箇所 mechanical rename
- test 全て `FontContext::new()` (system font 経路) で pass するはず (determinism 不要)

### 8.3 `raikiri/tests/hello_world_vrt.rs` の更新

```rust
// Fetch 未実行時の early panic
let fonts_dir = Path::new("target/wpt/fonts");
assert!(fonts_dir.exists(),
    "target/wpt/fonts not found — run scripts/wpt/fetch.sh first");
assert!(fonts_dir.join("Lato-Regular.ttf").exists(),
    "Lato-Regular.ttf missing under target/wpt/fonts — WPT pin may need bump");

// build_wpt_font_ctx + html_to_png_with_fonts
let font_ctx = raikiri_dom::fonts::build_wpt_font_ctx(fonts_dir)
    .expect("build_wpt_font_ctx Ok");
let png_bytes = raikiri::html_to_png_with_fonts(input, font_ctx)
    .expect("html_to_png_with_fonts Ok");
// 以降 golden compare は既存 flow (Tolerance::EXACT)
```

Golden PNG (`crates/raikiri/tests/reference/hello-world/expected/page-0000.png`) を
Lato 経由の "Hi" real text で再生成。既存の
`RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt` フロー経由。

### 8.4 Cross-machine determinism 検証

- **(D1) 手動 verify** — 別マシン (別 OS / fontconfig 環境) で
  `cargo test -p raikiri --test hello_world_vrt` 実行、md5 一致確認
- (D2) docker container multi-image cross-verify は **本 issue の scope 外**
  ([[raikiri-spike-rcf]] 枠で後追い)
- 検証エビデンスは本 issue の close comment に "verified on X, Y machines,
  md5 = ...." で残す

### 8.5 Regression pins

- §8.1 の `preferred_first_orders_lato_before_ahem` と
  `generic_serif_resolves_to_registered_family` が §5, §7 decision の regression pin
- hello-world VRT 自身が cross-machine determinism の end-to-end regression pin

## 9. Rollout / Migration

### 9.1 Milestone 内順序 (単一 issue で完結、branch は worktree-e93-wpt-font-pin)

1. **scripts/wpt/ 追加** — fulgur から copy-adapt、pin fulgur 値、`fetch.sh` 実行
   で `target/wpt/fonts/` 生成 (developer 実行、CI 統合は後追い)
2. **raikiri-dom::fonts 追加** — API + walker + generic alias + FontError、unit test 8 個
3. **layout_single_page signature 変更** — 4th arg `font_ctx: FontContext` を追加、
   caller 12 箇所 update (production 1: `raikiri/src/html_to_png.rs:47` + test 11:
   raikiri-paint 5 + raikiri-dom layout.rs test 6)
4. **raikiri::html_to_png_with_fonts 追加** — 既存 `html_to_png` は delegate 経由に
5. **hello_world_vrt.rs 更新** — early panic + build_wpt_font_ctx +
   html_to_png_with_fonts、golden 再生成 (`RAIKIRI_UPDATE_GOLDENS=1`)
6. **README 更新** — hello-world fixture の "Lato 経由 visual" と scripts/wpt/ 使い方

### 9.2 CI 影響

- CI に `scripts/wpt/fetch.sh` step 追加 (別 issue [[raikiri-spike-rcf]] 枠)
- cache key に `pinned_sha.txt` を含める
- 本 issue の scope 内では CI 変更 0、developer prerequisite として fetch を明記

### 9.3 Cross-project impact

- fulgur との pin drift 監視 (fulgur pin を bump したら raikiri も追従判断)
- blitz `packages/blitz-dom/src/lib.rs::build_single_font_ctx` との shape drift 監視
  (blitz 側 API 変化があれば追従判断)

## 10. Open Questions

- Q10.1: WPT fetch step の CI 統合 timing — 本 issue の close 後 [[raikiri-spike-rcf]]
  に punt でよいか (本 issue の scope 内では local dev 手動実行のみ、CI 側の fetch
  step は別 issue で追加)
- Q10.2: pin bump のトリガー基準 — fulgur の pin bump に追従 auto-bump か、
  raikiri 側 test regression 発火まで freeze か (M2+ の運用で決める)
- Q10.3: hello-world 以外の VRT (M1.15 以降で登場する fixture) の font pin path —
  build_wpt_font_ctx を module 越しに再利用する形で自然に伝播するはず、明示 pin なし

## 11. References

- roborev job 253 finding (m1.14 hello-world VRT)
- fulgur `scripts/wpt/fetch.sh` (2026-04-21 pin `97ea26e2`)
- fulgur `crates/fulgur-wpt/src/fonts.rs::load_fonts_dir`
- blitz `packages/blitz-dom/src/lib.rs::build_single_font_ctx`
- blitz `packages/blitz-dom/src/document.rs:355-380` (font_ctx Config)
- parley 0.10 / fontique `Collection::register_fonts`, `append_generic_families`,
  `CollectionOptions { system_fonts }`
- raikiri-spike design doc (2026-07-13-raikiri-rebuild-design.md) §M1 "ASCII
  (Latin) 単一 script + 単一 font に限定"
