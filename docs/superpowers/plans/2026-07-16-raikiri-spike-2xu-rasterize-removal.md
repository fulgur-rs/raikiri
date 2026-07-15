# raikiri-spike-2xu: Delete `rasterize`, use `anyrender::render_to_buffer` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/raikiri-vrt/src/lib.rs` から `pub fn rasterize` を削除し、caller は `anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(paint, W, H)` を直接使う形に統一する。bug の shape (renderer 状態借用) を structurally 除去する。

**Architecture:** raikiri-vrt crate の役割を「tiny-skia PNG encoder」に純化。scene rasterize は anyrender の既存 fresh-renderer helper に委譲。blitz-paint の caller-owns-reset design と mental model を揃える。

**Tech Stack:** Rust 1.89, anyrender 0.11.1, anyrender_vello_cpu 0.14.0, tiny-skia 0.11.4, peniko/kurbo

## Global Constraints

- **MSRV**: Rust 1.89 (spec §13.0 drift log 参照、実 toolchain は 1.89)
- **No dep changes**: 新規 dep 追加や version bump なし。Cargo.toml の `description` フィールド (メタデータ) は Task 2 で lib.rs docstring と整合させる
- **cleanroom for blitz backport**: raikiri surface に stylo/blitz 実装型を持ち込まない (この issue は該当なし、dev-only crate)
- **Byte-identical semantics preserved**: anyrender::render_to_buffer は fresh renderer per call — determinism 特性は VelloCpuImageRenderer::new + render_to_vec と等価
- **beads workflow**: task tracking は bd のみ、TodoWrite/TaskCreate 使用禁止

---

## File Structure

**Modify only:** `crates/raikiri-vrt/src/lib.rs`

- 現在: crate docstring (行 1-14) + `pub fn rasterize` (行 34-41) + `pub fn encode_png` (行 43-71) + `#[cfg(test)] mod tests` (行 73-144)
- 変更後: crate docstring (encoder-only role 反映) + `pub fn encode_png` (unchanged) + `#[cfg(test)] mod tests` (4 tests, rasterize 参照ゼロ)

**No other files touched** (grep 確認済: 外部 caller = 0 件)。

---

## Task 1: Migrate tests to `anyrender::render_to_buffer` + add regression-intent test

**Files:**
- Modify: `crates/raikiri-vrt/src/lib.rs:73-144` (test module のみ、`rasterize` 関数は次 task で削除)

**Interfaces:**
- Consumes: `anyrender::render_to_buffer::<R, F>(draw_fn: F, width: u32, height: u32) -> Vec<u8>` (anyrender 0.11.1 の free function)
- Produces: 4 pass tests in `raikiri_vrt::tests` module — 後続 task が rasterize 削除しても引き続き通ること

### Steps

- [ ] **Step 1.1: 現行 3 tests を anyrender::render_to_buffer に書き換え**

  `crates/raikiri-vrt/src/lib.rs` の `#[cfg(test)] mod tests` の 3 tests を以下に置換:

  ```rust
  #[test]
  fn render_helper_produces_expected_buffer_shape() {
      #[allow(clippy::redundant_closure)]
      let buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_red_rect(scene),
          W,
          H,
      );
      assert_eq!(buf.len(), (W as usize) * (H as usize) * 4);
      // Pixel (50, 50) is inside the red rect; must not still be transparent-black.
      let idx = (50 * (W as usize) + 50) * 4;
      assert_ne!(
          &buf[idx..idx + 4],
          &[0, 0, 0, 0],
          "expected non-transparent pixel at (50,50); scene did not render",
      );
  }

  #[test]
  fn render_helper_is_byte_identical() {
      #[allow(clippy::redundant_closure)]
      let a = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_red_rect(scene),
          W,
          H,
      );
      #[allow(clippy::redundant_closure)]
      let b = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_red_rect(scene),
          W,
          H,
      );
      assert_eq!(a.len(), b.len());
      assert!(
          a == b,
          "independent renderers diverged: {} of {} bytes differ",
          a.iter().zip(&b).filter(|(x, y)| x != y).count(),
          a.len(),
      );
  }

  #[test]
  fn encode_png_produces_png_signature() {
      #[allow(clippy::redundant_closure)]
      let rgba = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_red_rect(scene),
          W,
          H,
      );
      let png = encode_png(&rgba, W, H);
      // PNG magic bytes: \x89 P N G \r \n \x1A \n
      assert_eq!(
          &png[..8],
          &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'],
          "PNG magic bytes mismatch — encoder produced non-PNG output",
      );
  }
  ```

  **重要**: test mod 冒頭の `use super::{encode_png, rasterize};` を `use super::encode_png;` に変更。`rasterize` import は不要になる。

  さらに `draw_red_rect` 関数の docstring から rasterize への言及を削除:

  ```rust
  /// Deliberately trivial scene: a solid axis-aligned red rectangle.
  /// If this is not deterministic then nothing else in the pipeline will be.
  ///
  /// Call sites wrap this in a closure (`|scene| draw_red_rect(scene)`) —
  /// necessary because Rust cannot infer `S` from a bare fn-item argument
  /// against `render_to_buffer`'s `FnOnce(&mut R::ScenePainter<'_>)` HRTB.
  /// The closure unifies its parameter type directly per call, so the
  /// accompanying `#[allow(clippy::redundant_closure)]` attributes are
  /// load-bearing.
  fn draw_red_rect<S: PaintScene>(scene: &mut S) { ... }
  ```

- [ ] **Step 1.2: 新規 test `two_different_scenes_render_independently` 追加**

  上記 3 tests の後に追加:

  ```rust
  #[test]
  fn two_different_scenes_render_independently() {
      fn draw_blue_rect<S: PaintScene>(scene: &mut S) {
          let color: Color = css::BLUE;
          let rect = Rect::new(0.0, 0.0, 50.0, 50.0);
          scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
      }

      // Render red rect first, then blue rect via a separate fresh render. Under
      // the m1.8-era `rasterize(&mut R, ...)` bug (renderer state accumulation),
      // the blue buffer would still contain the red rect from the prior render.
      // With `anyrender::render_to_buffer`'s fresh-renderer-per-call semantics,
      // that state leak is structurally impossible — this test documents that
      // invariant.
      #[allow(clippy::redundant_closure)]
      let _red_buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_red_rect(scene),
          W,
          H,
      );
      #[allow(clippy::redundant_closure)]
      let blue_buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
          |scene| draw_blue_rect(scene),
          W,
          H,
      );

      // Pixel (75, 75) is inside red rect [10..90, 10..90] but OUTSIDE blue rect
      // [0..50, 0..50]. Under bug: blue_buf would still contain red rect at
      // (75,75) → non-transparent. Under fix: blue_buf is a fresh render of blue
      // only → transparent-black.
      let idx = (75 * (W as usize) + 75) * 4;
      let pixel = &blue_buf[idx..idx + 4];
      assert_eq!(
          pixel,
          &[0, 0, 0, 0],
          "blue_buf(75,75) not transparent — red rect from prior render leaked (state accumulation bug regression). Got: {pixel:?}",
      );

      // Sanity: pixel (25, 25) is inside blue rect — must be non-transparent.
      let idx2 = (25 * (W as usize) + 25) * 4;
      let pixel2 = &blue_buf[idx2..idx2 + 4];
      assert_ne!(
          pixel2,
          &[0, 0, 0, 0],
          "blue_buf(25,25) transparent — blue rect did not render at all",
      );
  }
  ```

- [ ] **Step 1.3: 4 tests 全て pass することを検証**

  Run: `cargo test -p raikiri-vrt`

  Expected output:
  ```
  running 4 tests
  test tests::render_helper_produces_expected_buffer_shape ... ok
  test tests::render_helper_is_byte_identical ... ok
  test tests::encode_png_produces_png_signature ... ok
  test tests::two_different_scenes_render_independently ... ok

  test result: ok. 4 passed; 0 failed; 0 ignored; ...
  ```

  **注意**: この時点で `pub fn rasterize` はまだ存在するが、test からは呼ばれない (`use super::encode_png` に変更したので `rasterize` は unused warning が出る可能性)。次 task で削除するので許容。

- [ ] **Step 1.4: clippy + fmt check**

  Run:
  ```bash
  cargo clippy -p raikiri-vrt --all-targets -- -D warnings
  cargo fmt --check -p raikiri-vrt
  ```

  Expected: both clean. `rasterize` が dead_code warning 出す可能性 → その場合は次 task の削除で解消するので、この step では `#[allow(dead_code)]` を rasterize に一時付与するか、そのまま次 task へ進むか判断。**方針**: 一時 allow は付けない、warning が出たら Step 1.5 で commit せず Task 2 に直進する。dead_code warning が出ない可能性が高い (pub 関数は unused warning 出ない)。

- [ ] **Step 1.5: commit**

  ```bash
  git add crates/raikiri-vrt/src/lib.rs
  git commit -m "$(cat <<'EOF'
  test(raikiri-vrt): migrate tests to anyrender::render_to_buffer + add regression-intent test

  Rewrites 3 existing tests (rasterize_*, encode_png_*) to use
  anyrender::render_to_buffer::<VelloCpuImageRenderer, _> instead of the
  local rasterize wrapper. Adds two_different_scenes_render_independently
  which documents that renderer-state accumulation (the m1.8-era bug) is
  now structurally impossible under fresh-renderer-per-call semantics.

  rasterize function itself is deleted in the follow-up commit.

  Refs: raikiri-spike-2xu
  EOF
  )"
  ```

---

## Task 2: Delete `pub fn rasterize` + update crate docstring

**Files:**
- Modify: `crates/raikiri-vrt/src/lib.rs:1-41` (crate docstring + rasterize 関数)
- Modify: `crates/raikiri-vrt/Cargo.toml:4` (`description` field を encoder-only role に整合)

**Interfaces:**
- Consumes: (Task 1 で移行済みの test suite)
- Produces: rasterize が存在しない raikiri-vrt crate。encode_png は unchanged

### Steps

- [ ] **Step 2.1: `pub fn rasterize` 関数を削除**

  `crates/raikiri-vrt/src/lib.rs` の以下ブロックを削除 (現在行 18-41):

  ```rust
  /// Rasterize an `anyrender` scene into an RGBA8 pixel buffer via the supplied
  /// backend.
  ///
  /// The `paint` closure receives `&mut R::ScenePainter` and issues draw
  /// commands via the `anyrender::PaintScene` trait.
  ///
  /// # Buffer layout
  ///
  /// `width * height * 4` bytes, tightly packed rows, premultiplied RGBA8
  /// (the `anyrender_vello_cpu` convention that `encode_png` accepts).
  ///
  /// # Panics
  ///
  /// Panics if the supplied renderer panics (e.g. any backend that rejects
  /// zero-dimension buffers at construction time). The wrapper adds no
  /// additional validation; VRT usage prefers fail-fast over `Result`.
  pub fn rasterize<R: ImageRenderer>(
      renderer: &mut R,
      paint: impl for<'a> FnOnce(&mut R::ScenePainter<'a>),
  ) -> Vec<u8> {
      let mut buf = Vec::new();
      renderer.render_to_vec(paint, &mut buf);
      buf
  }
  ```

  同時に、削除により不要になった `use anyrender::ImageRenderer;` (行 16) も削除。

- [ ] **Step 2.2: crate docstring を encoder-only role に書き換え**

  現在の `lib.rs:1-14` (crate docstring 全体) を以下に置換:

  ```rust
  //! raikiri-vrt — VRT harness (tiny-skia PNG encoder for anyrender pipelines).
  //!
  //! Thin wrapper providing the last-mile of the M1 render pipeline:
  //!   RGBA8 buffer → PNG bytes.
  //!
  //! Scene rasterization is handled by `anyrender::render_to_buffer` (fresh
  //! renderer per call, matches blitz-paint's caller-owns-reset convention).
  //! Backend selection: caller passes the concrete `ImageRenderer` type via
  //! turbofish. M1 default = `anyrender_vello_cpu::VelloCpuImageRenderer`.
  //!
  //! Downstream consumers:
  //! - `raikiri` umbrella `html_to_png` (M1 end-to-end pipeline)
  //! - hello-world VRT (m1.14)
  //! - determinism test (m1.13)
  //! - rayon thread-count test (m1.18)
  ```

- [ ] **Step 2.3a: Cargo.toml `description` フィールド書き換え**

  現在 `crates/raikiri-vrt/Cargo.toml:4`:
  ```toml
  description = "VRT (visual regression testing) harness for raikiri: anyrender rasterizer + tiny-skia PNG encoder"
  ```

  変更後:
  ```toml
  description = "VRT (visual regression testing) harness for raikiri: tiny-skia PNG encoder for anyrender pipelines"
  ```

- [ ] **Step 2.3: `cargo test -p raikiri-vrt` 全 pass を再確認**

  Run: `cargo test -p raikiri-vrt`

  Expected:
  ```
  running 4 tests
  test tests::render_helper_produces_expected_buffer_shape ... ok
  test tests::render_helper_is_byte_identical ... ok
  test tests::encode_png_produces_png_signature ... ok
  test tests::two_different_scenes_render_independently ... ok

  test result: ok. 4 passed; 0 failed; 0 ignored; ...
  ```

- [ ] **Step 2.4: clippy + fmt clean を再確認**

  Run:
  ```bash
  cargo clippy -p raikiri-vrt --all-targets -- -D warnings
  cargo fmt --check -p raikiri-vrt
  ```

  Expected: 両方 clean (warning ゼロ)。

- [ ] **Step 2.5: 外部 caller ゼロ再確認**

  Run: `grep -rn "raikiri_vrt::rasterize\|use.*raikiri_vrt.*rasterize" . --include="*.rs" 2>/dev/null | grep -v "target/\|worktrees/\|\.claude/"`

  Expected: **無出力** (0 件)。

- [ ] **Step 2.6: commit**

  ```bash
  git add crates/raikiri-vrt/src/lib.rs crates/raikiri-vrt/Cargo.toml
  git commit -m "$(cat <<'EOF'
  refactor(raikiri-vrt)!: delete rasterize wrapper, defer to anyrender::render_to_buffer

  Removes the local rasterize(&mut R, paint) wrapper. Its state-borrowing
  signature was the root cause of raikiri-spike-2xu: repeated calls with
  one renderer accumulated prior scene commands. anyrender 0.11.1 already
  provides render_to_buffer::<R, _>(paint, W, H) with fresh-renderer-per-
  call semantics that structurally prevent this bug.

  Aligns raikiri-vrt with blitz-paint's caller-owns-reset design (blitz-
  paint 0.3.0-beta.1 render.rs:119 — `// scene.reset();` intentionally
  commented out) — reduces impedance for future blitz backport.

  Crate role is now purely tiny-skia PNG encoding (encode_png). External
  callers = 0 (grep verified); test suite migrated in prior commit.

  Closes: raikiri-spike-2xu
  EOF
  )"
  ```

---

## Verification checklist (final)

Task 2 完了後、以下を確認:

- [ ] `pub fn rasterize` が lib.rs に存在しない (`grep 'pub fn rasterize' crates/raikiri-vrt/src/lib.rs` → 空)
- [ ] `use anyrender::ImageRenderer;` が lib.rs に存在しない (rasterize 削除で不要)
- [ ] test 4 件全 pass (`cargo test -p raikiri-vrt`)
- [ ] clippy clean (`cargo clippy -p raikiri-vrt --all-targets -- -D warnings`)
- [ ] fmt clean (`cargo fmt --check -p raikiri-vrt`)
- [ ] git log 2 commits (test migration + rasterize deletion)
- [ ] worktree branch = `worktree-raikiri-spike-2xu`

Session close protocol の一環として beads issue close:

```bash
bd close raikiri-spike-2xu --reason="deleted rasterize (bug shape removed); anyrender::render_to_buffer replaces it"
```

その後 finishing-a-development-branch skill で merge / PR 判断。
