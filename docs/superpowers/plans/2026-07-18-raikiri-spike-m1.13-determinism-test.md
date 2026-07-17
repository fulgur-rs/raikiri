# raikiri-spike-m1.13 Determinism Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `raikiri::html_to_png` を同一 process 内で 10 回連続実行しても byte-identical output を返すことを CI で verify する integration test を追加する。

**Architecture:** 単一 test file (`crates/raikiri/tests/hello_world_determinism.rs`) を新規追加。既存 hello-world fixture (`crates/raikiri/tests/reference/hello-world/input.html`) を reuse し、iter 0 の output を base として iter 1..10 と `Vec<u8>` を直接 byte 比較。mismatch 時は iter index / 両 length / first-differ offset を含む panic メッセージを出す。新 dep は追加しない。

**Tech Stack:** Rust integration test (`#[test]` in `crates/raikiri/tests/`)、`std::fs`、`std::path::PathBuf`、`raikiri::html_to_png` (m1.14 で追加された public API)。

## Global Constraints

- 新 dep 追加禁止: raikiri crate の `Cargo.toml` (`[dependencies]` / `[dev-dependencies]`) を変更しない (design AC #6)
- Test 実行時間 < 30 秒 on Linux x86_64 (spec §12.6 「数十秒」/ design AC #7)
- Iteration count: 10 (spec §12.6 / design AC #3)
- Determinism scope は same-process のみ。Subprocess / Rayon threads / Cross-arch は M8 送り (design "却下した option" 参照)
- Fixture は既存 `crates/raikiri/tests/reference/hello-world/input.html` を reuse (design "Fixture / Input")

---

## File Structure

**新規作成:**
- `crates/raikiri/tests/hello_world_determinism.rs` — determinism integration test 単一 file

**変更なし:**
- `crates/raikiri/tests/reference/hello-world/input.html` — 既存 fixture を reuse
- `crates/raikiri/Cargo.toml` — dep 追加なし

---

## Task 1: Add hello_world_determinism.rs integration test

**Files:**
- Create: `crates/raikiri/tests/hello_world_determinism.rs`

**Interfaces:**
- Consumes: `raikiri::html_to_png(&[u8]) -> Result<Vec<u8>, raikiri::RenderError>` (m1.14 で確立した public API)
- Produces: `#[test] fn hello_world_is_byte_identical_across_10_runs()` — cargo test target。他 task/crate から call されない

### - [ ] Step 1: Write the test file

Create `crates/raikiri/tests/hello_world_determinism.rs` with the following content:

```rust
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

    let iter0 = raikiri::html_to_png(&input).expect("iter 0 html_to_png must succeed");

    for iter in 1..ITERATIONS {
        let iter_n = raikiri::html_to_png(&input)
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
```

### - [ ] Step 2: Verify the file compiles

Run: `cargo build -p raikiri --tests`

Expected: exit code 0, no warnings that reference `hello_world_determinism.rs`. (既存 warning があれば無視、この file 由来の warning が無いことのみ確認)

### - [ ] Step 3: Run the new test and verify it passes

Run: `cargo test -p raikiri --test hello_world_determinism -- --nocapture`

Expected:
```
running 1 test
test hello_world_is_byte_identical_across_10_runs ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in <T>s
```

`<T>` は 30 秒未満であること (design AC #7 / spec §12.6 「数十秒」)。もし fail した場合:
- panic message 内の iter index / offset を control として、raikiri 内部で non-deterministic な source (`HashMap` iter, unpinned FontContext state, `Instant::now()` 由来の値, 未 sort な parallel merge, etc) を疑う
- **scope creep 警告**: fail が発見された場合は m1.13 の scope を超える bug。別 bd issue を切って fix、m1.13 は open のまま残す判断が必要 (ユーザーに escalate)

### - [ ] Step 4: Verify no regression on the full raikiri crate test suite

Run: `cargo test -p raikiri`

Expected: 既存 test (build_cascaded, external_consumer, hello_world_vrt, internal unit tests) が全て pass。合計 test 数が baseline (`hello_world_determinism` 追加前) + 1 になる。

### - [ ] Step 5: Verify Cargo.toml is untouched

Run: `git diff crates/raikiri/Cargo.toml`

Expected: 空 diff (design AC #6)

### - [ ] Step 6: Commit

```bash
git add crates/raikiri/tests/hello_world_determinism.rs
git commit -m "$(cat <<'EOF'
test(raikiri): hello-world determinism VRT (m1.13)

raikiri::html_to_png を同一 process 内で 10 回連続実行し、全 output が
iter 0 と byte-identical であることを verify する integration test を
追加。spec §12.3 / §12.6 の M1 acceptance criteria (Byte-identical 10
回実行) を満たす。

- 単一 test file crates/raikiri/tests/hello_world_determinism.rs を追加
- 既存 hello-world fixture (tests/reference/hello-world/input.html) を
  reuse、新 fixture / 新 dep 追加なし
- Mismatch 時は iter index / 両 length / first-differ byte offset を
  含む panic メッセージ (sha2 等の hash dep なし、direct Vec<u8> 比較)

Determinism testing matrix の他次元 (spec §12.8: Rayon threads /
Process / Arch/OS / Fonts / Dep upgrade) は M8 の
cross-thread-cross-arch-cross-os-determinism-tests task に defer。
本 test は same-process base-case のみ担う。

Refs: raikiri-spike-m1.13
EOF
)"
```

Expected: commit 作成成功、pre-commit hook 通過 (fmt / clippy 等)。

---

## Task 2: Update beads issue metadata

**Files:**
- Modify: beads issue `raikiri-spike-m1.13` (via `bd` CLI、file 変更なし)

**Interfaces:**
- Consumes: Task 1 の commit
- Produces: なし (metadata 更新のみ)

### - [ ] Step 1: Verify all acceptance criteria are met

Manual check against design AC:

| AC | Verification |
|---|---|
| 1. hello_world_determinism.rs 新規作成 | `test -f crates/raikiri/tests/hello_world_determinism.rs` |
| 2. #[test] fn hello_world_is_byte_identical_across_10_runs 実装 | `grep -q "fn hello_world_is_byte_identical_across_10_runs" crates/raikiri/tests/hello_world_determinism.rs` |
| 3. html_to_png を 10 回 call、iter 0 と == 一致 | Task 1 Step 3 pass |
| 4. Panic に iter index / 両 length / first-differ offset | source を目視、`grep -q "first differing byte at offset" crates/raikiri/tests/hello_world_determinism.rs` |
| 5. cargo test -p raikiri --test hello_world_determinism pass on Linux x86_64 | Task 1 Step 3 pass |
| 6. Cargo.toml 変更なし | Task 1 Step 5 空 diff |
| 7. 実行時間 < 30 秒 | Task 1 Step 3 の `finished in` 数字を目視 |

全 AC 満たしていない場合は該当 step に戻る。

### - [ ] Step 2: Report completion status

セッション終了時の handoff で以下を報告する予定 (この step 自体は実行不要、`superpowers:finishing-a-development-branch` skill が扱う):
- 変更 file: `crates/raikiri/tests/hello_world_determinism.rs` (1 file 新規)
- Branch: `worktree-raikiri-spike-m1.13`
- Test: `cargo test -p raikiri --test hello_world_determinism` pass
- Determinism testing matrix の他次元 (M8 送り) の記録は既に design "却下した option" section に含まれるため、追加 issue 起票不要

---

## Self-Review Notes (writing-plans skill Self-Review checklist の結果)

**1. Spec coverage:**
- design "Scope": Task 1 Step 1 の test 実装で cover
- design "Fixture / Input" (hello-world reuse): Task 1 Step 1 の `input_path` で cover
- design "Comparison / Diagnostics" (byte 比較 + iter/len/offset panic): Task 1 Step 1 の `assert_pngs_byte_identical` で cover
- design "Test 構造": Task 1 Step 1 の実装が概念コードを具体化
- design "却下した option": commit message に記録 (Task 1 Step 6)
- design AC 1-7: Task 2 Step 1 で verification checklist 化

Gap なし。

**2. Placeholder scan:** "TBD" / "similar to Task N" / "add error handling" 系無し。Task 1 Step 3 で fail 時の対応は "escalate" と明示 (silent placeholder ではなく operational instruction)。

**3. Type consistency:**
- `raikiri::html_to_png(&[u8]) -> Result<Vec<u8>, RenderError>` を Task 1 で consume、Task 1 内で完結
- `assert_pngs_byte_identical(&[u8], &[u8], usize)` を Task 1 内で define & call、シグネチャ一致
- 名前は plan / commit message 全体で `hello_world_is_byte_identical_across_10_runs` と `assert_pngs_byte_identical` に統一

Consistency issue なし。
