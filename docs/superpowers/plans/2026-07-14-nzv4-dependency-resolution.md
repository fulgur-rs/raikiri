# nzv.4 dependency-resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** M0 の dependency-resolution gate として、現状 untracked の `Cargo.lock` (nzv.3 完了時点の manifests に対応) を verify pass ののち git tracked に確定し、workspace の version 解決健全性を record する。

**Architecture:** 既存 lockfile を in-place verify する 3 段の cargo verify (V1: `cargo metadata --frozen`, V2: `cargo tree -d`, V3: `cargo check --workspace`) を実施 → selectors/cssparser の resolved version snapshot を追加取得 → beads notes に結果一式を record → `Cargo.lock` を single commit で tracked 化。深い selectors↔cssparser 整合 verify は nzv.10 に委譲。

**Tech Stack:** cargo 1.89 (rust-toolchain.toml pinned)、workspace resolver = "3"、外部 dep は blitz-consistent version (cssparser 0.37 / selectors 0.39 / html5ever 0.39 / markup5ever 0.39 / taffy 0.12 / parley 0.10 / anyrender 0.11 / peniko 0.6 / kurbo 0.13 / anyrender_vello_cpu 0.14 / blitz-{traits,html,dom,paint} 0.3.0-beta.1)。

## Global Constraints

- 対象 lockfile: 本 worktree に既に配置済みの `Cargo.lock` (main tree の untracked コピーと byte-identical、SHA-256 `9665c35a8f61d6cf7c310ab9d77878c9ae79d1c677f4bbafed42b35bd336b123`)。
- toolchain: `rust-toolchain.toml` の `1.89.0` を使用 (channel 差異による resolve 差分を排除)。
- 全 verify command は project root (`.` = このワークツリーの top) から実行。
- `cargo generate-lockfile` は使用しない (「in-place verify」方針、regenerate は Approach B として棄却済)。
- fail 時の fallback は design doc §Risk & Fallback に従う (manifest pin ずれなら nzv.3 に diff で戻し、feature flag 起因の Non-goal fail なら notes に記録して close)。
- notes 記録先は beads issue `raikiri-spike-nzv.4` の `--notes` フィールド。`docs/feasibility-report.md` は §M0「全 spike に依存」に従い今回は触らない。

---

## File Structure

**Modify (git tracked に追加):**
- `Cargo.lock` (workspace root、既に worktree に配置済み・untracked)

**Modify (beads DB):**
- issue `raikiri-spike-nzv.4` の `notes` フィールド (via `bd update`)

**Scratchpad (worktree 外、成果物ではない):**
- `/tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-*.log`

**依存する既存 file (変更しない):**
- `Cargo.toml` (workspace root、nzv.3 で確定)
- `crates/*/Cargo.toml` (nzv.3 で確定)
- `rust-toolchain.toml` (nzv.1 で確定、edce05d で MSRV 1.89 に bump)

---

### Task 1: Verification V1/V2 + selectors/cssparser snapshot

**Files:**
- Read: `Cargo.lock`, `Cargo.toml`, `rust-toolchain.toml`
- Write: `<scratchpad>/nzv4-verify-v1.log`, `<scratchpad>/nzv4-verify-v2.log`, `<scratchpad>/nzv4-verify-tree-css.log`

**Interfaces:**
- Consumes: 前提として `Cargo.lock` が worktree root に存在 (byte-identical に main tree からコピー済)。
- Produces: 3 つの log ファイル (Task 3 の notes 集約が消費)、および V1/V2 の pass 判定 (Task 2 が gate として使用)。

- [ ] **Step 1: Toolchain + lockfile 存在確認**

Run:
```bash
rustc --version && cargo --version && ls -la Cargo.lock rust-toolchain.toml
```
Expected:
- `rustc 1.89.0 (...)` — `rust-toolchain.toml` の channel と一致
- `cargo 1.89.0 (...)`
- `Cargo.lock` が 69226 bytes 前後で存在、`rust-toolchain.toml` に `channel = "1.89.0"`

- [ ] **Step 2: V1 実行 (cargo metadata --frozen)**

Run:
```bash
cargo metadata --frozen --format-version=1 > /dev/null 2>&1
echo "exit=$?"
```
Expected: `exit=0` (`--frozen` は Cargo.lock を更新しない mode、resolver error があれば非 0)。

- [ ] **Step 3: V1 の human-readable 再実行 (log 用)**

Run:
```bash
cargo metadata --frozen --format-version=1 2>&1 | jq -r '.workspace_members[]' \
  > /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v1.log
wc -l /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v1.log
```
Expected: 10 行 (workspace member 10 crate 全てが列挙される)。

- [ ] **Step 4: V2 実行 (cargo tree -d)**

Run:
```bash
cargo tree --workspace --edges normal -d 2>&1 \
  | tee /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v2.log
echo "exit=$?"
```
Expected: `exit=0`。log 内容は empty (duplicate なし) または「同 crate の複数 version が graph 上に存在」の list。empty が理想、非 empty の場合は Task 3 の notes で明示的に扱う。

- [ ] **Step 5: selectors/cssparser snapshot 取得**

Run:
```bash
cargo tree -p selectors -p cssparser --edges normal 2>&1 \
  | tee /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-tree-css.log
```
Expected: `selectors v0.39.x` と `cssparser v0.37.x` それぞれが単一 version で列挙される。複数 version 出現時は Task 3 の notes で挙げて nzv.10 (feasibility-selectors-cssparser-version-compat) へ引き継ぐ。

- [ ] **Step 6: Task 1 のコミットは無し**

このタスクは verify データ収集のみ。git tracked file の変更はしない。lockfile の commit は Task 3 で単発実施。

---

### Task 2: Verification V3 (workspace compile-check)

**Files:**
- Read: workspace 全 crate の `Cargo.toml`, `src/lib.rs`
- Write: `<scratchpad>/nzv4-verify-v3.log`

**Interfaces:**
- Consumes: Task 1 の V1/V2 pass を前提 (resolve が閉じているのを確認済のうえで compile に進む)。
- Produces: V3 の exit code と compile summary。Task 3 notes 集約が消費、design doc §Acceptance criterion 3 を satisfy する evidence。

- [ ] **Step 1: V3 実行 (cargo check --workspace --all-targets)**

Run:
```bash
cargo check --workspace --all-targets 2>&1 \
  | tee /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v3.log
echo "exit=$?"
```
Expected: `exit=0`。log 末尾に `Finished` が出力され、`warning:` は M0 で許容 (root workspace lint は `missing_docs = "warn"` のため stub crate では出うる)。`error:` が 0 件であること。

- [ ] **Step 2: V3 の error / warning 数を count**

Run:
```bash
grep -cE '^error' /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v3.log
grep -cE '^warning' /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-verify-v3.log
```
Expected: error 数 = 0。warning 数はいくつあってもよい (数字は Task 3 notes に記録)。

- [ ] **Step 3: fail 時の分岐判定**

- V3 が非 0 exit の場合:
  - error log を熟読し、原因が `Cargo.toml` の pin 誤り (nzv.3 の deliverable 由来) か feature flag 誤りかを判定。
  - 前者なら Task 3 を中止して nzv.3 に diff 対応を差し戻す (design doc §Risk & Fallback)。
  - 後者 (Non-goal 起因) なら Task 3 で notes に記録のうえ V3 を「Non-goal で fail」扱いで留め、design doc §Risk & Fallback の後段方針に従い close 可能性を評価。
- exit 0 なら Task 3 に進む。

- [ ] **Step 4: Task 2 のコミットは無し**

Task 1 と同様、verify データ収集のみ。

---

### Task 3: beads notes 更新 + Cargo.lock commit

**Files:**
- Read: 4 log ファイル `/tmp/claude-1000/.../scratchpad/nzv4-verify-*.log`
- Modify (git): `Cargo.lock` (untracked → tracked)
- Modify (beads): issue `raikiri-spike-nzv.4` の `notes` フィールド

**Interfaces:**
- Consumes: Task 1, Task 2 で生成された log と exit code。
- Produces: git commit 1 本 (`Cargo.lock` を track)、beads notes に verify 結果一式を記録。

- [ ] **Step 1: Notes 文書を組み立て**

以下フォーマットで scratchpad の `nzv4-notes.md` を作成:

```markdown
## Verification Results (nzv.4)

**Environment:**
- rustc: <output of `rustc --version`>
- cargo: <output of `cargo --version`>
- rust-toolchain.toml channel: 1.89.0
- Cargo.lock SHA-256: 9665c35a8f61d6cf7c310ab9d77878c9ae79d1c677f4bbafed42b35bd336b123

**V1 — cargo metadata --frozen:**
- exit: 0
- workspace members enumerated: 10
  - raikiri-traits, raikiri-style, raikiri-net, raikiri-dom, raikiri-html,
    raikiri-paint, raikiri, raikiri-blitz-compat, raikiri-wpt, raikiri-vrt

**V2 — cargo tree --workspace --edges normal -d:**
- exit: 0
- duplicate entries: <count>
- <if empty: "no duplicates">
- <if non-empty: 各 crate 名 + version を列挙。「nzv.10 へ引き継ぎ」と補足>

**V3 — cargo check --workspace --all-targets:**
- exit: 0
- errors: 0
- warnings: <count> (M0 では missing_docs 起因の warning を許容)

**selectors / cssparser snapshot:**
- selectors: v0.39.x (single version)
- cssparser: v0.37.x (single version)
- source tree excerpt:
  <cargo tree -p selectors -p cssparser 出力の抜粋>
```

`<...>` は Task 1 / Task 2 の log から埋める。

- [ ] **Step 2: beads notes を update**

Run:
```bash
bd update raikiri-spike-nzv.4 --notes "$(cat /tmp/claude-1000/-home-mitz-Work-oss-raikiri-spike/57b91891-acbf-42ca-8f63-75f987de1dee/scratchpad/nzv4-notes.md)"
bd show raikiri-spike-nzv.4 | grep -A 3 "NOTES" | head -20
```
Expected: `bd show` の出力に `NOTES` セクションが表示され、環境情報と 3 段 verify 結果が含まれる。

- [ ] **Step 3: Cargo.lock を stage**

Run:
```bash
git status Cargo.lock
git add Cargo.lock
git status Cargo.lock
```
Expected: `git add` 前は "Untracked"、後は "new file: Cargo.lock"。

- [ ] **Step 4: 意図しない stage が無いか確認**

Run:
```bash
git status
git diff --staged --stat
```
Expected: staged file が `Cargo.lock` のみ。`.beads/interactions.jsonl` の diff が worktree 側にあってはならない (worktree 内の bd 実行は main tree の DB を触るため、worktree の tracked file にはならない想定)。

- [ ] **Step 5: Commit**

Run:
```bash
git commit -m "$(cat <<'EOF'
build: freeze Cargo.lock for M0 dependency-resolution (nzv.4)

M0 dependency-resolution gate: freeze the workspace lockfile after in-place
verification. Confirms cargo metadata --frozen, cargo tree -d, and
cargo check --workspace --all-targets all pass on 1.89.0. selectors 0.39
and cssparser 0.37 each resolve to a single version; deep API compat is
delegated to nzv.10 (feasibility-selectors-cssparser-version-compat).
EOF
)"
git log --oneline -1
```
Expected: single commit `build: freeze Cargo.lock for M0 dependency-resolution (nzv.4)`、`git log --oneline -1` がそれを表示。

- [ ] **Step 6: Sanity check post-commit**

Run:
```bash
git ls-files | grep -x Cargo.lock
git status
```
Expected:
- `git ls-files` が `Cargo.lock` を返す (tracked 状態)。
- `git status` は clean、または beads/interactions.jsonl のような bd 副作用のみ。

---

## Self-Review 結果

- **Spec coverage:** design の acceptance criteria 全 6 項目に task を割り付け済 (1: Task 1 Step 2/3、2: Task 1 Step 4 + Task 3 Step 1、3: Task 2、4: Task 1 Step 5、5: Task 3 Step 3–5、6: Task 3 Step 2 + design 保存済)。
- **Placeholder scan:** TBD / TODO / "similar to Task N" 無し。全 code block は実行可能なフルコマンド。
- **Type consistency:** command 名は 3 タスク間で一貫 (V1/V2/V3、scratchpad ファイル名も一致)。
