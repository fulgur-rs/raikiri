# raikiri-spike-e6w: encode_png を raikiri umbrella に inline、raikiri-vrt を dev-dep 化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `raikiri` (publishable) が `raikiri-vrt` (`publish = false`) を runtime dep として消費している構造を解消する。`encode_png` を `raikiri::html_to_png` module 内に inline し、`raikiri-vrt` は VRT harness dev-dep として残す。

**Architecture:** `raikiri-vrt::encode_png` (tiny-skia PNG encoder) を `crates/raikiri/src/html_to_png.rs` の private helper に inline。`raikiri-vrt` を `raikiri` の `[dependencies]` から `[dev-dependencies]` に移動 (hello_world_vrt.rs integration test で dev-dep として引き続き使用)。`raikiri-vrt::reference` は自身 `tiny_skia::Pixmap::decode_png` を使うため tiny-skia dep は raikiri-vrt に残る。crate 数増加なし。

**Tech Stack:** Rust 2024 edition、Cargo workspace、tiny-skia (PNG encode/decode)、anyrender + anyrender_vello_cpu (rasterize)

## Global Constraints

- Rust edition: 2024 (workspace pin)
- rust-version: 1.89.0 (workspace pin)
- code motion + Cargo.toml edits のみ。`encode_png` 関数の logic は 1 文字も変更しない (behavior 保存)
- baseline: `cargo test --workspace --no-fail-fast` は 27 test result group / 合計 312 test passed (2026-07-18 wt baseline)。Task 完了後は 311 (encode_png_produces_png_signature -1 削除分)
- `raikiri-vrt/Cargo.toml` の `tiny-skia` dep は残す (`reference::decode_png` 用)
- 参照: [[blg-attribute-wiring-design]] の m1.14 hello-world VRT pipeline は本変更で機能的変化なし

---

## File Structure

| File | 責務 | 変更種別 |
|---|---|---|
| `crates/raikiri/Cargo.toml` | umbrella crate manifest | Modify: `[dependencies]` に `tiny-skia` 追加、`raikiri-vrt` を `[dependencies]` → `[dev-dependencies]` |
| `crates/raikiri/src/html_to_png.rs` | production HTML→PNG pipeline | Modify: `use raikiri_vrt::encode_png` 削除、private `encode_png` を module 内に inline |
| `crates/raikiri-vrt/src/lib.rs` | VRT harness crate root | Modify: `encode_png` を `pub` → `pub(crate)` に narrow (`reference::build_and_write_diff` 内部 caller 用)、`encode_png_produces_png_signature` test 削除、crate-level docstring 更新 |
| `crates/raikiri-vrt/tests/reference_harness.rs` | integration test harness | Modify: `use raikiri_vrt::encode_png` を削除、inline copy を追加 (integration test crate は `pub(crate)` が visible ではないため unavoidable) |

**Interfaces (task 間で必要な signature):**

- `fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8>` — `tiny_skia::Pixmap::encode_png` の thin wrapper。inline 後は `raikiri::html_to_png` module の private fn。behavior 保存 (byte-identical、3 copy 全て mod visibility 同一)

---

## Task 1: raikiri umbrella に encode_png を inline

**Files:**
- Modify: `crates/raikiri/Cargo.toml`
- Modify: `crates/raikiri/src/html_to_png.rs`

**Interfaces:**
- Consumes: なし (self-contained)
- Produces: `raikiri::html_to_png` の `encode_png` private helper (raikiri crate 内でのみ visible)

- [ ] **Step 1: `crates/raikiri/Cargo.toml` に tiny-skia dep を追加**

`[dependencies]` の末尾 (`anyrender_vello_cpu` の次) に追加:

```toml
# encode_png 実装用 (raikiri-spike-e6w で raikiri-vrt から inline)。
# reference::decode_png は raikiri-vrt に残るため tiny-skia は両 crate で使う。
tiny-skia           = { workspace = true }
```

- [ ] **Step 2: `crates/raikiri/Cargo.toml` の raikiri-vrt を `[dev-dependencies]` に移動**

`[dependencies]` セクションから以下 3 行を削除 (直前の comment "# m1.14 html_to_png: raster pipeline 用..." の説明を anyrender 側に絞る形で更新):

```toml
# 削除する行
# m1.14 html_to_png: raster pipeline 用。raikiri-vrt は encode_png、
# anyrender/anyrender_vello_cpu は render_to_buffer + VelloCpuImageRenderer。
raikiri-vrt         = { workspace = true }
```

comment は `[dependencies]` 側で以下に置き換え:

```toml
# m1.14 html_to_png: raster pipeline 用 (anyrender_vello_cpu で render_to_buffer)。
```

新規に `[dev-dependencies]` セクションを追加 (無ければ)。**重要**: workspace pin (`version = "0.1.0", path = ...`) を継承せず、path-only で指定する:

```toml
[dev-dependencies]
# hello_world_vrt integration test 用。raikiri-vrt は publish=false のため
# workspace pin (version = "0.1.0", path = ...) を継承せず path-only で指定する
# (workspace = true にすると version が manifest に残り、cargo publish 時に
#  path を strip した後 crates.io からの resolution が失敗するため)。
# path-only dev-dep は cargo publish 時に完全に strip される。
raikiri-vrt = { path = "../raikiri-vrt" }
```

- [ ] **Step 3: `crates/raikiri/src/html_to_png.rs` の import 差し替え**

削除:

```rust
use raikiri_vrt::encode_png;
```

代わりに import なし (encode_png は同 module 内の private fn になる)。

- [ ] **Step 4: encode_png を html_to_png.rs module 末尾に inline**

module 末尾 (`#[cfg(test)] mod tests` の**直前**) に追加:

```rust
/// Encode a premultiplied RGBA8 buffer to PNG bytes via `tiny_skia::Pixmap`.
///
/// The buffer must be exactly `width * height * 4` bytes. Buffer format is
/// premultiplied RGBA8 — the `anyrender_vello_cpu` output convention.
/// `tiny_skia` stores pixmaps in the same format, so encoding is a direct
/// wrap-then-serialize.
///
/// # Panics
///
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization failure (tiny-skia never returns an error for a
///   well-formed pixmap in practice; treated as an invariant violation)
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let pixmap = tiny_skia::Pixmap::from_vec(rgba.to_vec(), size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)");
    pixmap
        .encode_png()
        .expect("encode_png: tiny_skia::Pixmap::encode_png should not fail for a valid pixmap")
}
```

**Note**: 関数 body は `raikiri-vrt/src/lib.rs` の元実装と byte-identical (docstring 込み)。visibility を `pub` から `fn` (private) に変更のみ。

- [ ] **Step 5: `raikiri` crate だけ test を走らせて regression がないことを確認**

Run: `cargo test -p raikiri 2>&1 | tail -30`

Expected:
- `test html_to_png::tests::html_to_png_returns_png_bytes_for_hello_world ... ok`
- `test html_to_png::tests::html_to_png_propagates_parse_error_from_io ... ok`
- `test result: ok. N passed; 0 failed`

PNG magic byte assertion は既存 test が pin し続けるので、encode_png の behavior 保存を integration level で verify。

- [ ] **Step 6: workspace 全体 build で intermediate state を verify**

Run: `cargo build --workspace 2>&1 | tail -20`

Expected: `Compiling` 成功、warning なし。raikiri-vrt には encode_png がまだ残っているが dead-code warning は出ない (`pub fn` は crate 外 (raikiri crate side) から consume されなくても dead code 扱いされない、tests 側の `use super::encode_png` は残っている)。

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri/Cargo.toml crates/raikiri/src/html_to_png.rs
git commit -m "$(cat <<'EOF'
refactor(raikiri): inline encode_png from raikiri-vrt into html_to_png module

raikiri (publishable) が raikiri-vrt (publish=false) を runtime dep として
消費していた構造を解消するための first step。encode_png を html_to_png.rs
の private fn に inline し、raikiri-vrt を dev-dep に降格。

- Add tiny-skia to raikiri [dependencies] (encode_png 実装用)
- Move raikiri-vrt from [dependencies] to [dev-dependencies]
  (hello_world_vrt.rs integration test で引き続き dev-dep として使用)
- Inline encode_png as private fn in html_to_png.rs (byte-identical body)

raikiri-vrt/src/lib.rs 側の encode_png fn 削除は次 commit (raikiri-spike-e6w)。

Refs: raikiri-spike-e6w
EOF
)"
```

---

## Task 2: raikiri-vrt::encode_png を pub → pub(crate) に narrow、docstring を dev-only crate として更新

**Files:**
- Modify: `crates/raikiri-vrt/src/lib.rs`
- Modify: `crates/raikiri-vrt/tests/reference_harness.rs` (integration test は `pub(crate)` が見えないため inline copy が必要 — Task 2 実施時に判明した不可避の side-effect)

**Interfaces:**
- Consumes: Task 1 が raikiri 側で自前 encode_png を持っている状態
- Produces: `raikiri-vrt` の public API surface は `pub mod reference` のみに絞られる (encode_png は `pub(crate)` に narrow され export されない)、crate-level 責務が「VRT harness only」に絞られる

**設計注記 — pub(crate) を選ぶ理由**:
`raikiri-vrt::reference::build_and_write_diff` (crates/raikiri-vrt/src/reference.rs:535) が `crate::encode_png(&out, w, h)` を内部呼び出ししているため、encode_png を完全に削除すると内部 caller が壊れる。`pub` → `pub(crate)` に narrow することで:
- Public API surface は削除される (`publish = false` crate の pub(crate) fn は crates.io export されない)
- 内部 caller `reference.rs` は unchanged (net-zero diff)
- `raikiri-vrt/src` 内で単一 source of truth が保たれる
- `tests/reference_harness.rs` は integration test crate なので `pub(crate)` が visible ではなく、inline copy を保持する (unavoidable 3rd copy)

- [ ] **Step 1: raikiri-vrt/src/lib.rs の encode_png 可視性を pub → pub(crate) に narrow、テストを削除**

変更対象:

1. crate-level docstring 全体 (Step 3 で書き換え)
2. `pub fn encode_png(...)` を `pub(crate) fn encode_png(...)` に narrow、docstring 末尾に "Retained as a `pub(crate)` helper for internal callers (`reference::build_and_write_diff`). Not part of the public API — production PNG encoding lives in `raikiri::html_to_png`." 段落を追加
3. `#[cfg(test)] mod tests` 内の `use super::encode_png;` を削除
4. `#[cfg(test)] mod tests` 内の `#[test] fn encode_png_produces_png_signature()` 全体を削除 (`raikiri::html_to_png::tests::html_to_png_returns_png_bytes_for_hello_world` が同旨の PNG magic byte assertion を保持しているため)

**残す**: `pub mod reference;` 宣言、`pub(crate) fn encode_png` (visibility narrow 後)、`#[cfg(test)] mod tests` 内の以下 5 tests:
- `render_helper_produces_expected_buffer_shape`
- `render_helper_is_byte_identical`
- `render_to_buffer_leaves_untouched_pixels_transparent`
- `rayon_thread_pin_produces_expected_buffer_shape`
- `rayon_thread_pin_1_vs_4_is_byte_identical`

および `draw_red_rect`, `render_with_threads` 等の test helper。

- [ ] **Step 2: 変更後の raikiri-vrt/src/lib.rs 構造を diff で確認**

変更後の file 構造は以下になる (docstring + `pub mod reference;` + `pub(crate) fn encode_png` + `#[cfg(test)] mod tests`):

```rust
//! (Step 3 で新規 docstring 挿入)

pub mod reference;

/// (encode_png docstring with pub(crate) rationale — Step 1 参照)
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    // body byte-identical to Task 1 の raikiri html_to_png::encode_png
}

#[cfg(test)]
mod tests {
    use anyrender::PaintScene;
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Rect};
    use peniko::{Color, Fill, color::palette::css};

    const W: u32 = 100;
    const H: u32 = 100;

    // ... (draw_red_rect, tests continue)
```

`use super::encode_png;` が削除され、encode_png fn は `pub(crate)` に narrow された状態を確認。

- [ ] **Step 3: crate-level docstring を dev-only VRT harness 用に書き換え**

`pub mod reference;` の直前に以下 docstring を追加:

```rust
//! raikiri-vrt — VRT harness for raikiri (dev-only).
//!
//! Provides the reference-image fixture / diff / tolerance harness via the
//! `reference` module, plus anyrender rasterize + rayon determinism tests
//! that guard the raster pipeline used by production PNG encoding.
//! Production PNG encoding itself lives in `raikiri::html_to_png` as an
//! inlined private helper.
//!
//! This crate is `publish = false` because `reference::Fixture` and friends
//! are test-scaffolding APIs, not production surface.
//!
//! Downstream consumers (all dev-only):
//! - hello-world VRT integration test — `raikiri/tests/hello_world_vrt.rs`
//! - determinism / rayon thread-count tests — this crate's own `#[cfg(test)]`
```

- [ ] **Step 4: raikiri-vrt tests が pass することを確認**

Run: `cargo test -p raikiri-vrt 2>&1 | tail -20`

Expected: 5 tests passed (encode_png_produces_png_signature 削除により合計 6 → 5)。

```
test tests::render_helper_produces_expected_buffer_shape ... ok
test tests::render_helper_is_byte_identical ... ok
test tests::render_to_buffer_leaves_untouched_pixels_transparent ... ok
test tests::rayon_thread_pin_produces_expected_buffer_shape ... ok
test tests::rayon_thread_pin_1_vs_4_is_byte_identical ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- [ ] **Step 5: workspace 全体 test が pass することを確認**

Run: `cargo test --workspace --no-fail-fast 2>&1 | tail -30`

Expected: 全 crate 合計で baseline に対し「-1 test (encode_png_produces_png_signature 削除)」の状態で pass。baseline 312 → 311 (27 test result groups)。

hello_world_vrt.rs integration test も pass (raikiri-vrt が dev-dep として引き続き consume されている)。

- [ ] **Step 6: acceptance: raikiri crate の runtime dep に publish=false crate が含まれないことを verify**

Run:

```bash
cargo metadata --format-version=1 --no-deps 2>/dev/null | \
  jq -r '.packages[] | select(.name=="raikiri") | .dependencies[] | select(.kind == null) | .name' | \
  grep -q "^raikiri-vrt$" && echo "FAIL: raikiri-vrt still in runtime deps" || echo "PASS: raikiri-vrt not in runtime deps"
```

Expected: `PASS: raikiri-vrt not in runtime deps`

補足 verify (tiny-skia が入っていること):

```bash
cargo metadata --format-version=1 --no-deps 2>/dev/null | \
  jq -r '.packages[] | select(.name=="raikiri") | .dependencies[] | select(.kind == null) | .name' | \
  grep -q "^tiny-skia$" && echo "PASS: tiny-skia in runtime deps" || echo "FAIL: tiny-skia missing"
```

Expected: `PASS: tiny-skia in runtime deps`

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri-vrt/src/lib.rs crates/raikiri-vrt/tests/reference_harness.rs
git commit -m "$(cat <<'EOF'
refactor(raikiri-vrt): narrow encode_png to pub(crate); dev-only VRT harness

encode_png は raikiri::html_to_png の private fn に inline 済 (前 commit)。
raikiri-vrt::encode_png は internal caller (reference::build_and_write_diff)
用に pub(crate) fn として残し、public API surface からは削除。crate-level
docstring を dev-only VRT harness として書き換え。

- Narrow encode_png visibility: pub → pub(crate) (docstring に rationale 追記)
- Remove encode_png_produces_png_signature test (raikiri::html_to_png::tests
  の html_to_png_returns_png_bytes_for_hello_world が同旨の PNG magic byte
  assertion を保持)
- Rewrite crate-level docstring to reflect dev-only VRT harness role
- Inline encode_png in tests/reference_harness.rs (integration test は
  pub(crate) が visible ではないため unavoidable な 3rd copy)

Refs: raikiri-spike-e6w
EOF
)"
```

**note**: 実装過程で Task 2 initial commit が `pub fn encode_png` を完全削除し、`reference::build_and_write_diff` の内部呼び出しが壊れたことが post-task review で判明。fix commit で `pub(crate)` に narrow し reference.rs は baseline に revert。本 Step 7 の commit message 例は最終形 (post-fix) を反映している。

---

## Self-Review Checklist (implementer 用)

Task 2 完了後、以下を verify:

- [ ] `grep -n "raikiri_vrt::encode_png\|use raikiri_vrt::encode_png" crates/raikiri/src/` — hit なし
- [ ] `grep -n "^pub fn encode_png" crates/raikiri-vrt/src/lib.rs` — hit なし (public API 削除)
- [ ] `grep -n "^pub(crate) fn encode_png" crates/raikiri-vrt/src/lib.rs` — hit 1 件 (internal helper 残存)
- [ ] `grep -n "fn encode_png" crates/raikiri/src/html_to_png.rs` — hit 1 件 (private fn)
- [ ] `grep -n "fn encode_png" crates/raikiri-vrt/tests/reference_harness.rs` — hit 1 件 (inline copy)
- [ ] `git diff <baseline>..HEAD -- crates/raikiri-vrt/src/reference.rs` — 空 (net-zero、reference module 未変更)
- [ ] `git diff <baseline>..HEAD -- crates/raikiri-vrt/Cargo.toml` — 空 (tiny-skia dep 保持)
- [ ] `cargo build --workspace` — clean (0 warnings)
- [ ] `cargo test --workspace` — pass (delta: -1 test = encode_png_produces_png_signature 削除分)
- [ ] `cargo metadata` acceptance — raikiri-vrt が runtime dep にない、tiny-skia がある
- [ ] `crates/raikiri/tests/hello_world_vrt.rs` は unchanged (git diff で verify)

## Non-goals

- `raikiri-vrt::reference` API 変更 (Fixture, Tolerance, run_and_compare, DiffReport — 全て surface 保持)
- Tolerance tier 値の変更 (spec §12.7 準拠のまま)
- raikiri-vrt の tiny-skia dep 削除 (`decode_png` 用に残す)
- 新規 crate (`raikiri-vrt-core`) の作成 — 却下済 Option A、design 参照
