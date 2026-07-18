# raikiri-spike-e6w: encode_png を raikiri umbrella に inline、raikiri-vrt を dev-dep 化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `raikiri` (publishable) が `raikiri-vrt` (`publish = false`) を runtime dep として消費している構造を解消する。`encode_png` を `raikiri::html_to_png` module 内に inline し、`raikiri-vrt` は VRT harness dev-dep として残す。

**Architecture:** `raikiri-vrt::encode_png` (tiny-skia PNG encoder) を `crates/raikiri/src/html_to_png.rs` の private helper に inline。`raikiri-vrt` を `raikiri` の `[dependencies]` から `[dev-dependencies]` に移動 (hello_world_vrt.rs integration test で dev-dep として引き続き使用)。`raikiri-vrt::reference` は自身 `tiny_skia::Pixmap::decode_png` を使うため tiny-skia dep は raikiri-vrt に残る。crate 数増加なし。

**Tech Stack:** Rust 2024 edition、Cargo workspace、tiny-skia (PNG encode/decode)、anyrender + anyrender_vello_cpu (rasterize)

## Global Constraints

- Rust edition: 2024 (workspace pin)
- rust-version: 1.89.0 (workspace pin)
- code motion + Cargo.toml edits のみ。`encode_png` 関数の logic は 1 文字も変更しない (behavior 保存)
- baseline: `cargo test --workspace` は 13 test result group / 合計 46 test passed (2026-07-18 wt baseline)
- `raikiri-vrt/Cargo.toml` の `tiny-skia` dep は残す (`reference::decode_png` 用)
- 参照: [[blg-attribute-wiring-design]] の m1.14 hello-world VRT pipeline は本変更で機能的変化なし

---

## File Structure

| File | 責務 | 変更種別 |
|---|---|---|
| `crates/raikiri/Cargo.toml` | umbrella crate manifest | Modify: `[dependencies]` に `tiny-skia` 追加、`raikiri-vrt` を `[dependencies]` → `[dev-dependencies]` |
| `crates/raikiri/src/html_to_png.rs` | production HTML→PNG pipeline | Modify: `use raikiri_vrt::encode_png` 削除、private `encode_png` を module 内に inline |
| `crates/raikiri-vrt/src/lib.rs` | VRT harness crate root | Modify: `pub fn encode_png` 削除、`encode_png_produces_png_signature` test 削除、crate-level docstring 更新 |

**Interfaces (task 間で必要な signature):**

- `fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8>` — `tiny_skia::Pixmap::encode_png` の thin wrapper。inline 後は `raikiri::html_to_png` module の private fn。behavior 保存 (byte-identical)

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

新規に `[dev-dependencies]` セクションを追加 (無ければ):

```toml
[dev-dependencies]
# hello_world_vrt integration test 用 (raikiri-spike-e6w: production は publishable なので dev-dep へ)
raikiri-vrt = { workspace = true }
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

## Task 2: raikiri-vrt から encode_png を削除、docstring を dev-only crate として更新

**Files:**
- Modify: `crates/raikiri-vrt/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 が raikiri 側で自前 encode_png を持っている状態
- Produces: `raikiri-vrt` は `pub mod reference` のみ export、crate-level 責務が「VRT harness only」に絞られる

- [ ] **Step 1: raikiri-vrt/src/lib.rs から encode_png fn とそのテストを削除**

削除対象:

1. crate-level docstring 全体 (Step 3 で書き換え)
2. `pub fn encode_png(...)` 全体 (docstring 込み、`/// Encode a premultiplied RGBA8 ...` から `}` まで)
3. `#[cfg(test)] mod tests` 内の `use super::encode_png;`
4. `#[cfg(test)] mod tests` 内の `#[test] fn encode_png_produces_png_signature()` 全体

**残す**: `pub mod reference;` 宣言、`#[cfg(test)] mod tests` 内の以下 5 tests:
- `render_helper_produces_expected_buffer_shape`
- `render_helper_is_byte_identical`
- `render_to_buffer_leaves_untouched_pixels_transparent`
- `rayon_thread_pin_produces_expected_buffer_shape`
- `rayon_thread_pin_1_vs_4_is_byte_identical`

および `draw_red_rect`, `render_with_threads` 等の test helper。

- [ ] **Step 2: 削除後の raikiri-vrt/src/lib.rs 冒頭を diff で確認**

削除後の file top 部分は以下になる (docstring + `pub mod reference;` + `#[cfg(test)] mod tests`):

```rust
//! (Step 3 で新規 docstring 挿入)

pub mod reference;

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

`use super::encode_png;` が削除されたことを確認。

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

Expected: 全 crate 合計で baseline に対し「-1 test (encode_png_produces_png_signature 削除)」の状態で pass。baseline 46 → 45。

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
git add crates/raikiri-vrt/src/lib.rs
git commit -m "$(cat <<'EOF'
refactor(raikiri-vrt): remove encode_png (moved to raikiri umbrella)

encode_png は raikiri::html_to_png の private fn に inline 済 (前 commit)。
raikiri-vrt は VRT harness 責務のみに絞り、reference module + rasterize/
rayon 決定性 tests に集約。crate-level docstring を dev-only VRT harness
として書き換え。

- Remove pub fn encode_png (docstring 込み)
- Remove encode_png_produces_png_signature test (raikiri::html_to_png::tests
  の html_to_png_returns_png_bytes_for_hello_world が同旨の PNG magic byte
  assertion を保持)
- Rewrite crate-level docstring to reflect dev-only VRT harness role

Refs: raikiri-spike-e6w
EOF
)"
```

---

## Self-Review Checklist (implementer 用)

Task 2 完了後、以下を verify:

- [ ] `grep -n "raikiri_vrt::encode_png\|use raikiri_vrt" crates/raikiri/src/` — hit なし
- [ ] `grep -n "pub fn encode_png\|fn encode_png" crates/raikiri-vrt/src/lib.rs` — hit なし
- [ ] `grep -n "fn encode_png" crates/raikiri/src/html_to_png.rs` — hit 1 件 (private fn)
- [ ] `cargo build --workspace` — clean (0 warnings)
- [ ] `cargo test --workspace` — pass (baseline 46 → 45)
- [ ] `cargo metadata` acceptance — raikiri-vrt が runtime dep にない、tiny-skia がある
- [ ] `crates/raikiri/tests/hello_world_vrt.rs` は unchanged (git diff で verify)

## Non-goals

- `raikiri-vrt::reference` API 変更 (Fixture, Tolerance, run_and_compare, DiffReport — 全て surface 保持)
- Tolerance tier 値の変更 (spec §12.7 準拠のまま)
- raikiri-vrt の tiny-skia dep 削除 (`decode_png` 用に残す)
- 新規 crate (`raikiri-vrt-core`) の作成 — 却下済 Option A、design 参照
