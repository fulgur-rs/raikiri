# m1.12 CI Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** GitHub Actions で `cargo fmt --check` + `cargo test --lib` + `cargo clippy` を PR/push で自動実行する CI baseline を確立する。

**Architecture:** 単一 job on `ubuntu-latest`、Rust toolchain は `rust-toolchain.toml` (1.89.0 pinned) を rustup に auto-install させる。`rustfmt.toml` に edition = "2024" を pin し、fmt-check を strict gate として運用。既存コードは `cargo fmt --all` で一括正規化し、fmt-check が最初から green で landed する状態を作る。

**Tech Stack:** GitHub Actions YAML、`actions/checkout@v4`、`Swatinem/rust-cache@v2`、pinned rustup toolchain、cargo (fmt / test / clippy)。

## Global Constraints

- 全 crate `edition = "2024"` (Cargo.toml workspace root で pin 済)
- Rust toolchain: `1.89.0` (rust-toolchain.toml + Cargo.toml rust-version)
- `Cargo.lock` は `--locked` で使用強制、CI で lock drift 検出
- Runner: `ubuntu-latest` のみ (macOS / Windows は m1.13 以降)
- Skip 対象 path: `docs/**`, `.beads/**`, root-level `*.md`
- 実行 command は必ず worktree ディレクトリで実行 (`/home/mitz/worktrees/github.com/mitsuru/raikiri-spike/worktree-raikiri-spike-m1.12`)

---

### Task 1: rustfmt.toml + codebase normalization

**Files:**
- Create: `rustfmt.toml` (repo root)
- Modify: 複数 file across `crates/**/*.rs` (mechanical `cargo fmt --all` 出力)

**Interfaces:**
- Consumes: 既存の Cargo.toml `edition = "2024"` (workspace)
- Produces: `cargo fmt --all -- --check` が exit 0 で通過する codebase 状態、
  Task 2 の CI workflow の fmt step が baseline で green になる前提を作る

- [ ] **Step 1: Create rustfmt.toml at repo root**

Create `rustfmt.toml`:

```toml
edition = "2024"
```

`rustfmt.toml` は 1 行 config。workspace 全 crate が `edition = "2024"` を継承しているのに合わせる。future の `imports_layout` / `group_imports` 等の style tuning は別 issue に defer。

- [ ] **Step 2: Verify baseline non-conformance**

Run: `cargo fmt --all -- --check`
Expected: exit code non-zero, diff 出力あり (raikiri-dom / raikiri-html / raikiri-traits 等の複数 file、design brainstorming 中に確認済)

これで「normalize pass が必要な initial state」を pin する。green で通過してしまう場合は Step 3〜5 skip して Step 6 の commit へ (rustfmt.toml 追加のみ)。

- [ ] **Step 3: Run cargo fmt --all**

Run: `cargo fmt --all`
Expected: exit code 0、複数 file が変更される (git status で確認)

`cargo fmt` は edition = 2024 の rustfmt が bundled される 1.89 toolchain で実行。
将来同 toolchain で reproducible な出力になる。

- [ ] **Step 4: Verify fmt-check now passes**

Run: `cargo fmt --all -- --check`
Expected: exit code 0、diff 出力なし

Task 2 の CI workflow の fmt-check step が baseline で通ることを事前保証。

- [ ] **Step 5: Verify test + clippy still green after normalize**

Run: `cargo test --workspace --lib --locked`
Expected: 190 tests passed (m1.12 baseline)

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: exit code 0、warning なし

fmt 正規化は semantic 変更を伴わない mechanical pass だが、macro 展開位置や
attribute の並び替え等で raw string / clippy lint に副作用が出る可能性を念のため
確認 (unlikely だが cheap check)。

- [ ] **Step 6: Commit**

```bash
git add rustfmt.toml crates/
git commit -m "$(cat <<'EOF'
chore(fmt): normalize codebase to rustfmt 2024 defaults + add rustfmt.toml (m1.12)

Prep for m1.12 CI fmt-check gate. rustfmt.toml pins edition = "2024" to
match workspace Cargo.toml edition (crate 全 edition.workspace 継承)。
cargo fmt --all を一度実行、raikiri-dom / raikiri-html / raikiri-traits 等
複数 file を rustfmt 2024 default 出力に統一。以後 CI で cargo fmt --check
が strict gate として機能する。

Semantic 変更なし: 190 workspace tests green、clippy -D warnings clean。

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe
EOF
)"
```

---

### Task 2: GitHub Actions ci.yml

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `rust-toolchain.toml` (Task 1 と無関係、既存)、Task 1 が landing した
  `rustfmt.toml` + normalized codebase
- Produces: PR / push to main で発火する CI job、`raikiri-spike-a6s` (BLOCKS
  relationship) を unblock

- [ ] **Step 1: Create .github/workflows/ci.yml**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
    paths-ignore:
      - 'docs/**'
      - '.beads/**'
      - '*.md'
  pull_request:
    paths-ignore:
      - 'docs/**'
      - '.beads/**'
      - '*.md'

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  ci:
    name: fmt + test + clippy
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup Rust toolchain
        # rustup が rust-toolchain.toml (1.89.0 + rustfmt + clippy) を尊重、
        # 初回 cargo 呼び出し前に auto-install する。two sources of truth を避け
        # dtolnay/rust-toolchain の explicit version は使わない。
        run: rustup show active-toolchain

      - name: Cache cargo registry + target
        uses: Swatinem/rust-cache@v2

      - name: Format check
        run: cargo fmt --all -- --check

      - name: Test (lib only, locked)
        # raikiri-vrt integration test は m1.10 reference harness 完了後、
        # 別 issue で --tests に昇格。--locked で Cargo.lock drift を弾く。
        run: cargo test --workspace --lib --locked

      - name: Clippy (all targets, deny warnings)
        run: cargo clippy --workspace --all-targets --locked -- -D warnings
```

`.github/` ディレクトリは新規作成。GitHub 側 workflow parser が自動で拾う。

- [ ] **Step 2: Verify YAML syntax locally**

Run:

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
```

Expected: exit code 0、出力なし

action YAML の syntax 検証は最低限 python yaml で行う (actionlint が入っていれば
それを使うが、依存を追加しない方針)。

- [ ] **Step 3: Simulate CI commands locally**

Run each step in order to confirm reproducibility:

```bash
cargo fmt --all -- --check
cargo test --workspace --lib --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all 3 exit 0。fmt no-diff / 190 tests passed / clippy no-warnings。

これで push to origin → GitHub Actions が同 command sequence を実行した時に
green で通ることが local で pre-verify される。

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "$(cat <<'EOF'
feat(ci): add GitHub Actions workflow with fmt + test + clippy (m1.12)

closes raikiri-spike-m1.12

- Trigger: push to main, PR to any target。docs / .beads / root-level *.md
  変更のみの PR は skip (paths-ignore) してリソース節約。
- Runner: ubuntu-latest 単一。macOS / Windows は byte-identical rendering
  の cross-OS 検証が未成立のため m1.13 以降。
- Toolchain: rust-toolchain.toml (1.89.0 pinned + rustfmt + clippy) を
  rustup が auto-install。dtolnay/rust-toolchain 経由の explicit version は
  避け、two sources of truth 化を回避 (MSRV bump 時に rust-toolchain.toml
  だけ触ればよい)。
- Cache: Swatinem/rust-cache@v2 で cargo registry + incremental target を
  再利用。cache warm 時の実行時間 1-2 分程度を soft target とする。
- Concurrency: workflow+ref group で cancel-in-progress。同 PR の連続 push で
  古い run が中断される。
- Steps: fmt --check (fail-fast) → test --lib --locked → clippy --locked。
  --locked で Cargo.lock drift 検出、build 単独 step は test が兼務するため
  省略。

Not included (defer):
- macOS / Windows job (m1.13)
- MSRV / stable / beta matrix (rust-toolchain.toml pin 済で意味薄)
- cargo doc build (public API 未固定)
- cargo audit (M2+ security review flow)
- raikiri-vrt integration test (m1.10 reference harness 待ち)
- Nightly regression

Unblocks: raikiri-spike-a6s (raikiri-wpt/lint quarantine ∩ baseline filter
overlap analysis) がこの workflow を前提とする。

Claude-Session: https://claude.ai/code/session_01Qo7TYgbGc4JQnghRS6nUJe
EOF
)"
```

---

## Post-plan verification hooks (out-of-band)

Plan 実行後、以下は skill の verification / branch finishing フェーズで行う:

- `git log --oneline main..HEAD` で 2 commits (Task 1 + Task 2) が並ぶことを確認
- `git diff main..HEAD --stat` で change surface が想定内 (rustfmt.toml、
  .github/workflows/ci.yml、crates/**/*.rs の formatting-only 変更) であることを
  確認
- push to origin 後、GitHub UI で workflow が発火し green で通ることを目視確認
  (post-merge に相当、m1.12 close 直前でよい)
