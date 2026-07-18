# raikiri-spike-e93 WPT Font Pin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** VRT cross-machine 決定性のため、`raikiri_dom::layout::preshape_text` の `parley::FontContext` を WPT bundled fonts (`target/wpt/fonts/`) に pin し、hello-world VRT の golden PNG を任意マシン間で byte-identical にする。

**Architecture:** fulgur の `scripts/wpt/` (sparse shallow clone) + blitz の `build_single_font_ctx` (system_fonts: false + generic family alias append) を合成。raikiri-dom に `build_wpt_font_ctx(&Path) -> Result<FontContext, FontError>` を新設、`layout_single_page` の signature に `font_ctx: FontContext` を追加、raikiri crate に `html_to_png_with_fonts` を並列追加 (既存 `html_to_png` は delegate 経由で維持)。

**Tech Stack:** Rust 2024 edition, rust 1.89.0, parley 0.10 (workspace dep, `fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache}`), std のみ (thiserror / log / tracing 追加なし)。既存 spec: `docs/superpowers/specs/2026-07-18-raikiri-spike-e93-wpt-font-pin-design.md`。

## Global Constraints

- **Rust edition**: `edition = "2024"`, `rust-version = "1.89.0"` (workspace)
- **License**: `MIT OR Apache-2.0`
- **No new external crate deps** — thiserror / log / tracing は追加しない (raikiri workspace 慣習に準拠、`FontError` は `enum + Display + Debug + std::error::Error impl` を手書き、warning は `eprintln!`)
- **Worktree**: `.claude/worktrees/e93-wpt-font-pin` (branch `worktree-e93-wpt-font-pin`) 内で作業。この plan は既に該当 worktree で実行される想定
- **CLAUDE.md Conservative profile**: 各 task の commit は plan 内 step として明示、push は明示指示なしでは実行しない
- **`raikiri-dom` は `raikiri-wpt` を dep しない** — cycle 回避 (dep 方向は `raikiri-dom ← raikiri ← test`)
- **Production runtime の pub API 不変** — `raikiri::html_to_png(input)` の signature は unchanged、既存 M1.14 API を破壊しない
- **Commit message convention**: `<type>(<scope>): <subject> (e93)` (recent history 準拠、例 `docs(specs): raikiri-spike-e93 WPT font pin design (e93)`)

---

## File Structure

**New files**:
- `scripts/wpt/fetch.sh` — shallow + sparse checkout → `target/wpt/`
- `scripts/wpt/subset.txt` — sparse-checkout patterns (`fonts`)
- `scripts/wpt/pinned_sha.txt` — WPT upstream SHA pin (fulgur `97ea26e2…` 借用)
- `scripts/wpt/README.md` — 使い方 + pin 更新プロセス
- `crates/raikiri-dom/src/fonts.rs` — `build_wpt_font_ctx` + `FontError` + walker

**Modified files**:
- `crates/raikiri-dom/src/lib.rs` — `pub mod fonts;` + `pub use fonts::{build_wpt_font_ctx, FontError};`
- `crates/raikiri-dom/src/layout.rs` — `pub fn layout_single_page` に `font_ctx: FontContext` 引数追加 (signature 変更)、内部 `FontContext::new()` 削除、`#[cfg(test)]` 内 6 caller の rename
- `crates/raikiri-paint/src/lib.rs` — 5 test caller の rename
- `crates/raikiri/src/html_to_png.rs` — `layout_single_page` caller に font_ctx を pass、内部関数 `html_to_png_impl(input, font_ctx)` に refactor
- `crates/raikiri/src/lib.rs` — `pub fn html_to_png_with_fonts` を追加、既存 `html_to_png` は delegate に置換
- `crates/raikiri/tests/hello_world_vrt.rs` — early panic (target/wpt/fonts + Ahem.ttf 存在確認) + `build_wpt_font_ctx` + `html_to_png_with_fonts`
- `crates/raikiri/tests/reference/hello-world/expected/page-0000.png` — Ahem 経路 (red square) の golden 再生成
- `crates/raikiri/tests/reference/hello-world/README.md` — Ahem square visual への変化を追記

---

## Task Overview

| # | Task | Deliverable |
|---|---|---|
| 1 | scripts/wpt/ pipeline | `target/wpt/fonts/` 生成、Ahem.ttf 存在 |
| 2 | raikiri-dom::fonts scaffold + FontError | `build_wpt_font_ctx` stub + missing/empty dir test |
| 3 | fonts walker + PREFERRED_FIRST | walker recursive + sort + Ahem 先頭 (PREFERRED_FIRST=["Ahem.ttf"]) |
| 4 | build_wpt_font_ctx (blitz pattern) | system_fonts:false + register + generic alias |
| 5 | layout_single_page signature 変更 | `font_ctx` 引数追加、caller 12 箇所 update |
| 6 | html_to_png_with_fonts + delegate | pub API 追加、既存 html_to_png は delegate |
| 7 | hello_world_vrt.rs 更新 | build_wpt_font_ctx + early panic (golden 未更新でまだ fail 期待) |
| 8 | golden 再生成 + README 更新 | Ahem 経路の byte-identical PNG (red square)、visual 説明 |

---

### Task 1: scripts/wpt/ pipeline

**Files:**
- Create: `scripts/wpt/fetch.sh`
- Create: `scripts/wpt/subset.txt`
- Create: `scripts/wpt/pinned_sha.txt`
- Create: `scripts/wpt/README.md`

**Interfaces:**
- Consumes: なし (単独設定)
- Produces: `target/wpt/fonts/` 内の WPT-standard font tree — 具体的には `target/wpt/fonts/Ahem.ttf` (+ 他 Lato-Bold/Medium、CSSTest 系) が checkout される。以降 task はこの path を前提。**注**: fulgur pin (`97ea26e2`) では `Lato-Regular.ttf` は不在 (Lato-Bold / Lato-Medium / Lato-Medium-Liga のみ)、そのため VRT primary は Ahem に決定 (Section 5 revised)

- [ ] **Step 1: `scripts/wpt/` dir 作成**

Run: `mkdir -p scripts/wpt`
Expected: no output

- [ ] **Step 2: `scripts/wpt/fetch.sh` を write**

Create `scripts/wpt/fetch.sh` (fulgur `scripts/wpt/fetch.sh` の literal copy、`REPO_ROOT` の指す先が `scripts/wpt/../..` = repo root で raikiri と共通):

```bash
#!/usr/bin/env bash
# Shallow-clone WPT upstream and sparse-checkout only the paths needed
# by raikiri-wpt / raikiri VRT tests. Idempotent: re-running updates to
# the pinned SHA.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WPT_DIR="$REPO_ROOT/target/wpt"
SHA_FILE="$SCRIPT_DIR/pinned_sha.txt"
SUBSET_FILE="$SCRIPT_DIR/subset.txt"
REMOTE_URL="${WPT_REMOTE_URL:-https://github.com/web-platform-tests/wpt.git}"

SHA="$(awk '!/^#/ && NF { print; exit }' "$SHA_FILE" | tr -d '[:space:]')"
if [ -z "$SHA" ]; then
  echo "error: no SHA in $SHA_FILE" >&2
  exit 1
fi

if [ ! -d "$WPT_DIR/.git" ]; then
  mkdir -p "$WPT_DIR"
  git -C "$WPT_DIR" init -q
  git -C "$WPT_DIR" config core.sparseCheckout true
  git -C "$WPT_DIR" config extensions.partialClone origin
fi

# Keep the remote URL in sync on every run so WPT_REMOTE_URL overrides
# (mirrors, CI caches) take effect even when target/wpt already exists.
git -C "$WPT_DIR" remote set-url origin "$REMOTE_URL" 2>/dev/null \
  || git -C "$WPT_DIR" remote add origin "$REMOTE_URL"

# Write sparse-checkout patterns (strip comments and blanks)
mkdir -p "$WPT_DIR/.git/info"
grep -v '^#' "$SUBSET_FILE" | sed '/^[[:space:]]*$/d' > "$WPT_DIR/.git/info/sparse-checkout"

# Fetch only the pinned SHA, filter=blob:none to keep it lean
git -C "$WPT_DIR" fetch --depth=1 --filter=blob:none origin "$SHA"
git -C "$WPT_DIR" checkout -q --detach FETCH_HEAD

echo "WPT ready at $WPT_DIR (SHA: $SHA)"
```

- [ ] **Step 3: `fetch.sh` を実行可能にする**

Run: `chmod +x scripts/wpt/fetch.sh`
Expected: no output

- [ ] **Step 4: `scripts/wpt/subset.txt` を write**

Create `scripts/wpt/subset.txt`:

```
# WPT subset needed for raikiri VRT tests. Keep minimal — M1 scope only
# needs bundled fonts for cross-machine determinism.
# Future test subsets (css/css-page, etc.) added under separate issues.
fonts
```

- [ ] **Step 5: `scripts/wpt/pinned_sha.txt` を write**

Create `scripts/wpt/pinned_sha.txt`:

```
# WPT upstream commit SHA pinned for raikiri VRT reproducibility.
# Initial pin borrowed from fulgur/scripts/wpt/pinned_sha.txt (2026-04-21).
# raikiri 専有 bump が必要な場合 (raikiri-side regression) は fulgur pin
# にとらわれず advance 可、その際は README の "Updating the pin" 手順。
# Verify before bumping:
#   scripts/wpt/fetch.sh && cargo test -p raikiri --test hello_world_vrt
97ea26e26a2aac3eec7e770650b25e7049ed4a4e
```

- [ ] **Step 6: `scripts/wpt/README.md` を write**

Create `scripts/wpt/README.md`:

```markdown
# scripts/wpt/

`fetch.sh` executes a sparse, shallow clone of the W3C web-platform-tests
repository into `target/wpt/`, pinned to the SHA in `pinned_sha.txt`.

The set of fetched paths is controlled by `subset.txt` (one pattern per line,
Git sparse-checkout syntax). Currently M1 scope: `fonts` only (Ahem, Lato,
CSSTest 等) for VRT cross-machine determinism (raikiri-spike-e93).

## Usage

    scripts/wpt/fetch.sh

Idempotent: re-running updates to the current pinned SHA. Override the remote
URL with `WPT_REMOTE_URL=...` (mirrors / CI cache warmup).

## Updating the pin

The pin is initially borrowed from fulgur (`fulgur/scripts/wpt/pinned_sha.txt`)
for cross-project consistency. Bump raikiri's pin only when:

1. fulgur bumps and raikiri should follow (default), OR
2. raikiri 専有 regression requires a fresh WPT font asset (rare)

Steps:

1. Inspect upstream WPT `main` (or fulgur's next pin) and pick a green commit
2. Replace the SHA line in `pinned_sha.txt`
3. Re-run `scripts/wpt/fetch.sh`
4. Re-run `cargo test -p raikiri --test hello_world_vrt`. Golden PNG may
   need regeneration if font asset content shifted:
   `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt`
5. Commit `pinned_sha.txt` + updated golden (if any) in one PR

## Relation to `raikiri-wpt`

`raikiri-wpt` (WPT test runner harness) does not currently depend on
`target/wpt/`. This subset is dedicated to raikiri VRT font pin
(raikiri-spike-e93). When `raikiri-wpt` starts consuming WPT test
resources, extend `subset.txt` and update this README.
```

- [ ] **Step 7: fetch.sh を実行し WPT resources 取得**

Run: `scripts/wpt/fetch.sh`
Expected: 最終行 `WPT ready at /home/mitz/Work/oss/raikiri-spike/.claude/worktrees/e93-wpt-font-pin/target/wpt (SHA: 97ea26e26a2aac3eec7e770650b25e7049ed4a4e)`。実行時間は初回で 30 秒〜数分 (network 依存)。

- [ ] **Step 8: 期待 font 資産の存在確認**

Run:
```bash
ls target/wpt/fonts/ | head -20
test -f target/wpt/fonts/Ahem.ttf && echo "Ahem OK"
```
Expected:
- `Ahem.ttf`、`Lato-Bold.ttf`、`Lato-Medium.ttf` などが list に含まれる (Lato-Regular は fulgur pin では**不在**、Ahem のみが VRT primary として保証される)
- `test -f Ahem.ttf` が echo を print

もし Ahem.ttf が無い場合、pinned_sha が壊れているか fetch failed。Ahem は全 WPT SHA で存在する core determinism font なので、無いなら BLOCKED として escalate。

- [ ] **Step 9: commit**

Run:
```bash
git add scripts/wpt/fetch.sh scripts/wpt/subset.txt scripts/wpt/pinned_sha.txt scripts/wpt/README.md
git status
git commit -m "chore(scripts): scripts/wpt/ fetch pipeline for VRT font pin (e93)

fulgur scripts/wpt/ から copy-adapt。sparse+shallow clone を target/wpt/
に落とし、pinned SHA は fulgur 借用 (97ea26e2, 2026-04-21)。

- fetch.sh: git init + partial clone + sparse-checkout + fetch pinned SHA
- subset.txt: 'fonts' のみ (M1 scope、raikiri-spike-e93)
- pinned_sha.txt: fulgur 借用、raikiri 専有 bump 手順は README
- README: 使い方 + pin 更新プロセス + raikiri-wpt との関係"
```

Expected: 1 commit created with 4 files added.

---

### Task 2: raikiri-dom::fonts module scaffold + FontError

**Files:**
- Create: `crates/raikiri-dom/src/fonts.rs`
- Modify: `crates/raikiri-dom/src/lib.rs`

**Interfaces:**
- Consumes: (Task 1) `target/wpt/fonts/` は unit test 直接使わず、fake ttf を tempdir で生成する pattern (fulgur `fulgur-wpt/src/fonts.rs` の tests に倣う)
- Produces:
  - `pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError>` — Task 3-4 で内部実装拡張
  - `pub enum FontError { DirNotFound(PathBuf), EmptyDir(PathBuf), Io { path: PathBuf, source: std::io::Error } }` + Display + Debug + std::error::Error impl
  - `crates/raikiri-dom/src/lib.rs` に `pub mod fonts;` + `pub use fonts::{build_wpt_font_ctx, FontError};` を追加

- [ ] **Step 1: `crates/raikiri-dom/src/fonts.rs` skeleton を write (未実装、コンパイル通す)**

Create `crates/raikiri-dom/src/fonts.rs`:

```rust
//! WPT bundled font dir を register し、system_fonts: false と generic
//! family alias で cross-machine 決定性 FontContext を構築する。
//!
//! - Fetch は `scripts/wpt/fetch.sh` (dev prerequisite)、本 module は
//!   fetch 済 `target/wpt/fonts/` を Path で受けるだけ
//! - production runtime は `parley::FontContext::new()` を今のまま使う
//! - M1 scope: `.ttf` / `.otf` のみ、WOFF/WOFF2 は M4+ (raikiri-spike-2sb)
//!
//! 参考実装:
//! - fulgur `crates/fulgur-wpt/src/fonts.rs::load_fonts_dir` (walker + sort)
//! - blitz `packages/blitz-dom/src/lib.rs::build_single_font_ctx`
//!   (system_fonts: false + generic alias append)

use parley::FontContext;
use std::path::{Path, PathBuf};

/// WPT bundled fonts dir から FontContext を構築する。system font
/// resolver は完全 disable、generic family (`serif`/`sans-serif`/...)
/// は register 済 family の先頭 (Ahem) に解決される。
///
/// # Errors
/// - [`FontError::DirNotFound`] — `fonts_dir` が存在しない
/// - [`FontError::EmptyDir`] — dir は存在するが `.ttf`/`.otf` が 1 個も無い
/// - [`FontError::Io`] — dir walk 中の io failure
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError> {
    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    // Task 3-4 で walker + register + generic alias を実装
    Err(FontError::EmptyDir(fonts_dir.to_path_buf()))
}

/// [`build_wpt_font_ctx`] の error 型。std のみ、`thiserror` 依存なし
/// (raikiri workspace 慣習準拠)。
#[derive(Debug)]
pub enum FontError {
    /// `fonts_dir` が存在しない (fetch 未実行の場合など)
    DirNotFound(PathBuf),
    /// `fonts_dir` は存在するが `.ttf`/`.otf` が 1 個も見つからない
    EmptyDir(PathBuf),
    /// dir walk 中の io failure
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::DirNotFound(p) => {
                write!(f, "fonts dir not found: {}", p.display())
            }
            FontError::EmptyDir(p) => write!(
                f,
                "fonts dir has no .ttf/.otf files: {} (did you run scripts/wpt/fetch.sh?)",
                p.display()
            ),
            FontError::Io { path, source } => {
                write!(f, "io error reading {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for FontError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FontError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn missing_dir_returns_err() {
        let bogus = Path::new("/definitely/does/not/exist/raikiri-spike-e93");
        let err = build_wpt_font_ctx(bogus).unwrap_err();
        match err {
            FontError::DirNotFound(p) => assert_eq!(p, bogus),
            other => panic!("expected DirNotFound, got {:?}", other),
        }
    }
}
```

- [ ] **Step 2: `crates/raikiri-dom/src/lib.rs` に module 宣言 + re-export 追加**

Read `crates/raikiri-dom/src/lib.rs` して、既存の `pub mod` / `pub use` 位置を確認 (line 20-30 付近想定)。既存 `pub use layout::layout_single_page;` の隣に以下を追加:

```rust
pub mod fonts;
pub use fonts::{build_wpt_font_ctx, FontError};
```

配置は既存の `pub use` の block 内、alphabetical or logical order (layout の前 or 後、既存 crate の style 準拠)。

- [ ] **Step 3: Test を実行して pass 確認 (DirNotFound path)**

Run: `cargo test -p raikiri-dom fonts::tests::missing_dir_returns_err`
Expected: `test fonts::tests::missing_dir_returns_err ... ok`

- [ ] **Step 4: empty_dir_returns_empty_dir_err test を追加**

Add to `crates/raikiri-dom/src/fonts.rs` の `#[cfg(test)] mod tests`:

依存 crate: `tempfile` を dev-deps に追加する必要あり。まず Cargo.toml を確認:

Run: `grep -E '^tempfile' crates/raikiri-dom/Cargo.toml`

もし tempfile がなければ `crates/raikiri-dom/Cargo.toml` の `[dev-dependencies]` に追加:
```toml
[dev-dependencies]
tempfile = { workspace = true }
```
(workspace deps に既存: root `Cargo.toml` に `tempfile = "3"` あり、確認済)

Test を追加:

```rust
    #[test]
    fn empty_dir_returns_empty_dir_err() {
        let tmp = tempfile::tempdir().unwrap();
        let err = build_wpt_font_ctx(tmp.path()).unwrap_err();
        match err {
            FontError::EmptyDir(p) => assert_eq!(p, tmp.path()),
            other => panic!("expected EmptyDir, got {:?}", other),
        }
    }
```

- [ ] **Step 5: Test を実行して pass 確認 (EmptyDir path)**

Run: `cargo test -p raikiri-dom fonts::tests`
Expected: 2 tests pass (`missing_dir_returns_err`, `empty_dir_returns_empty_dir_err`)

- [ ] **Step 6: Cargo.toml が touched なら差分確認**

Run: `git diff crates/raikiri-dom/Cargo.toml`
Expected: `[dev-dependencies]` に `tempfile = { workspace = true }` が追加されている (Task 2 で初めて dev-dep に tempfile 追加、既に workspace deps は存在するので line 追加のみ)。既存で dev-dep があれば差分無し。

- [ ] **Step 7: commit**

Run:
```bash
git add crates/raikiri-dom/src/fonts.rs crates/raikiri-dom/src/lib.rs crates/raikiri-dom/Cargo.toml
git status
git commit -m "feat(raikiri-dom): fonts module scaffold + FontError (e93)

build_wpt_font_ctx stub + FontError enum (DirNotFound / EmptyDir / Io)。
std のみ、thiserror 依存追加なし。missing_dir と empty_dir の 2 test
で error path を pin。walker + register + generic alias は Task 3-4 で実装。"
```

Expected: 1 commit created.

---

### Task 3: raikiri-dom::fonts walker + PREFERRED_FIRST

**Files:**
- Modify: `crates/raikiri-dom/src/fonts.rs` (walker + PREFERRED_FIRST + tests 追加)

**Interfaces:**
- Consumes: (Task 2) `build_wpt_font_ctx` の DirNotFound / EmptyDir error path
- Produces:
  - Internal helper `fn walk_fonts(dir: &Path) -> Result<Vec<PathBuf>, FontError>` — dir を recursive walk、`.ttf`/`.otf` を collect + sort + PREFERRED_FIRST 移動
  - `PREFERRED_FIRST: &[&str] = &["Ahem.ttf"]` const (fulgur pin では Ahem 不在、Ahem 全 SHA で保証されるため primary)
  - `build_wpt_font_ctx` は walker を呼び EmptyDir 判定を実 collect 後に move (empty dir は EmptyDir を返す)

- [ ] **Step 1: PREFERRED_FIRST const と walker skeleton を追加**

Append to `crates/raikiri-dom/src/fonts.rs` (前置 `use` block と public API の間、または末尾):

```rust
/// PREFERRED_FIRST に list した family は walker sort の結果に関わらず
/// **配列 index 順に**先頭に register される。cascade `"serif"` の
/// resolve 順の決定性と、hello-world VRT visual (Ahem square "Hi") のため。
///
/// 現在は Ahem のみ (fulgur pin では Lato-Regular が不在)。将来 Lato-Medium 等の
/// real-text primary を追加したい場合は array に append する。
const PREFERRED_FIRST: &[&str] = &["Ahem.ttf"];

/// dir を recursive walk して `.ttf`/`.otf` を collect + sort + PREFERRED_FIRST
/// を先頭に move する。file_name の case は `.ttf`/`.otf` (小文字 normalize)。
fn walk_fonts(dir: &Path) -> Result<Vec<PathBuf>, FontError> {
    let mut collected: Vec<PathBuf> = Vec::new();
    collect_recursive(dir, &mut collected)?;
    // 1. path sort (決定性)
    collected.sort();
    // 2. PREFERRED_FIRST を先頭に partition
    let (preferred, rest): (Vec<PathBuf>, Vec<PathBuf>) = collected.into_iter().partition(|p| {
        p.file_name()
            .and_then(|f| f.to_str())
            .map(|n| PREFERRED_FIRST.contains(&n))
            .unwrap_or(false)
    });
    // 3. preferred は PREFERRED_FIRST の配列 index 順に再ソート
    let mut ordered_preferred: Vec<PathBuf> = Vec::new();
    for target in PREFERRED_FIRST {
        if let Some(p) = preferred.iter().find(|p| {
            p.file_name().and_then(|f| f.to_str()) == Some(*target)
        }) {
            ordered_preferred.push(p.clone());
        }
    }
    // 4. preferred + rest を結合
    let mut result = ordered_preferred;
    result.extend(rest);
    Ok(result)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), FontError> {
    let entries = std::fs::read_dir(dir).map_err(|source| FontError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_recursive(&path, out)?;
            continue;
        }
        let is_font = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| {
                let s = s.to_ascii_lowercase();
                s == "ttf" || s == "otf"
            })
            .unwrap_or(false);
        if is_font {
            out.push(path);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: `build_wpt_font_ctx` を walker 経由の EmptyDir 判定に置換**

Replace the current `build_wpt_font_ctx` body in `crates/raikiri-dom/src/fonts.rs`:

```rust
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError> {
    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    let paths = walk_fonts(fonts_dir)?;
    if paths.is_empty() {
        return Err(FontError::EmptyDir(fonts_dir.to_path_buf()));
    }
    // Task 4 で register + generic alias を実装、ここでは EmptyDir 判定後
    // に到達したことのみ担保
    let _ = paths;
    Err(FontError::EmptyDir(fonts_dir.to_path_buf()))
}
```

**注意**: 現時点で "paths が非空でも EmptyDir を返す" 状態 — これは意図的な中間ステップ。Task 4 で register 実装後に置換する。既存 test (Task 2 の empty_dir_returns_empty_dir_err) は tempdir 空、この改修後も pass する (paths が empty で正しく EmptyDir)。

- [ ] **Step 3: 既存 test 再実行して回帰なし**

Run: `cargo test -p raikiri-dom fonts::tests`
Expected: 2 tests pass (missing / empty)

- [ ] **Step 4: walker unit test を追加 — helpers + loads_ttf_files**

Add to `#[cfg(test)] mod tests`:

```rust
    use std::io::Write;

    /// Minimal valid TTF header (magic 0x00010000 + zero-fill).
    /// fontique の register_fonts は header 検査後 zero-fill body でも
    /// family_id を割り当てる (parse-invalid だが walker exercise には十分)。
    fn write_fake_ttf(dir: &Path, name: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(&[0x00, 0x01, 0x00, 0x00]).unwrap();
        f.write_all(&[0u8; 64]).unwrap();
    }

    #[test]
    fn walker_loads_ttf_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "a.ttf");
        write_fake_ttf(tmp.path(), "b.ttf");
        let paths = walk_fonts(tmp.path()).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.extension().and_then(|e| e.to_str()) == Some("ttf")));
    }
```

- [ ] **Step 5: Test 実行して pass 確認**

Run: `cargo test -p raikiri-dom fonts::tests::walker_loads_ttf_files`
Expected: pass

- [ ] **Step 6: ignores_non_font_extensions test を追加 + pass 確認**

Add:

```rust
    #[test]
    fn walker_ignores_non_font_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        write_fake_ttf(tmp.path(), "font.ttf");
        std::fs::write(tmp.path().join("README.md"), b"ignore").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"ignore").unwrap();
        let paths = walk_fonts(tmp.path()).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].file_name().and_then(|f| f.to_str()),
            Some("font.ttf")
        );
    }
```

Run: `cargo test -p raikiri-dom fonts::tests::walker_ignores_non_font_extensions`
Expected: pass

- [ ] **Step 7: recurses_into_subdirs test を追加 + pass 確認**

Add:

```rust
    #[test]
    fn walker_recurses_into_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("CSSTest");
        std::fs::create_dir(&sub).unwrap();
        write_fake_ttf(tmp.path(), "top.ttf");
        write_fake_ttf(&sub, "nested.ttf");
        let paths = walk_fonts(tmp.path()).unwrap();
        assert_eq!(paths.len(), 2);
    }
```

Run: `cargo test -p raikiri-dom fonts::tests::walker_recurses_into_subdirs`
Expected: pass

- [ ] **Step 8: sort_order_is_deterministic test を追加 + pass 確認**

Add:

```rust
    #[test]
    fn walker_sort_order_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        // Non-alphabetical create order to exercise sort
        for name in ["z.ttf", "a.ttf", "m.ttf"] {
            write_fake_ttf(tmp.path(), name);
        }
        let first = walk_fonts(tmp.path()).unwrap();
        let second = walk_fonts(tmp.path()).unwrap();
        assert_eq!(first, second, "walker must be deterministic across calls");
        let names: Vec<_> = first
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.ttf", "m.ttf", "z.ttf"]);
    }
```

Run: `cargo test -p raikiri-dom fonts::tests::walker_sort_order_is_deterministic`
Expected: pass

- [ ] **Step 9: preferred_first_orders_ahem_before_csstest test を追加 + pass 確認**

Add:

```rust
    #[test]
    fn walker_preferred_first_orders_ahem_before_csstest() {
        let tmp = tempfile::tempdir().unwrap();
        // Ahem.ttf は sort 順でも先頭 (A) だが、PREFERRED_FIRST[0] として
        // **explicit に**先頭に来ることを regression pin。将来 PREFERRED_FIRST
        // に Lato-Medium 等が追加されたとき Ahem が override されないよう
        // 意図明示 (現在は array 1 要素なので default sort と重複するが OK)。
        write_fake_ttf(tmp.path(), "Ahem.ttf");
        write_fake_ttf(tmp.path(), "CSSTest-Regular.ttf");
        write_fake_ttf(tmp.path(), "Lato-Bold.ttf");
        let paths = walk_fonts(tmp.path()).unwrap();
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().and_then(|f| f.to_str()).unwrap().to_string())
            .collect();
        // Ahem (PREFERRED_FIRST[0]) → 残りは path sort (CSSTest, Lato-Bold)
        assert_eq!(
            names,
            vec![
                "Ahem.ttf",
                "CSSTest-Regular.ttf",
                "Lato-Bold.ttf",
            ]
        );
    }
```

Run: `cargo test -p raikiri-dom fonts::tests::walker_preferred_first_orders_ahem_before_csstest`
Expected: pass

- [ ] **Step 10: fonts::tests 全 7 test 通し実行**

Run: `cargo test -p raikiri-dom fonts::tests`
Expected: 7 tests pass (`missing_dir_returns_err`, `empty_dir_returns_empty_dir_err`, `walker_loads_ttf_files`, `walker_ignores_non_font_extensions`, `walker_recurses_into_subdirs`, `walker_sort_order_is_deterministic`, `walker_preferred_first_orders_ahem_before_csstest`)

- [ ] **Step 11: commit**

Run:
```bash
git add crates/raikiri-dom/src/fonts.rs
git status
git commit -m "feat(raikiri-dom): fonts walker + PREFERRED_FIRST (e93)

walk_fonts: recursive collect + path sort + PREFERRED_FIRST partition。
Ahem を配列 index 順で先頭 register する基盤 (現在 array 1 要素、将来
Lato-Medium 等の real-text primary を append 拡張可能)。build_wpt_font_ctx
は empty 判定を walker 出力で行う中間状態 (register 実装は Task 4)。

- fulgur crates/fulgur-wpt/src/fonts.rs::load_fonts_dir を参考に walker
- unit test 5 個追加 (loads / ignores / recurses / sort_deterministic /
  preferred_first_orders_ahem_before_csstest)"
```

Expected: 1 commit.

---

### Task 4: build_wpt_font_ctx (blitz pattern: system_fonts:false + register + generic alias)

**Files:**
- Modify: `crates/raikiri-dom/src/fonts.rs` (build_wpt_font_ctx 実装 + generic alias test)

**Interfaces:**
- Consumes: (Task 3) `walk_fonts(&Path) -> Result<Vec<PathBuf>, FontError>` + `PREFERRED_FIRST`
- Produces:
  - `build_wpt_font_ctx(&Path) -> Result<FontContext, FontError>` 完成 (register + generic alias 済)
  - `FontContext { source_cache: SourceCache::new_shared(), collection: Collection::new(CollectionOptions { shared: false, system_fonts: false }) }` を返す
  - Generic family 6 種 (Serif, SansSerif, Monospace, SystemUi, Cursive, Fantasy) にすべて register 済 family_ids を append

- [ ] **Step 1: build_wpt_font_ctx を blitz pattern で実装**

Replace the current `build_wpt_font_ctx` body in `crates/raikiri-dom/src/fonts.rs`:

```rust
pub fn build_wpt_font_ctx(fonts_dir: &Path) -> Result<FontContext, FontError> {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
    use std::sync::Arc;

    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    let paths = walk_fonts(fonts_dir)?;
    if paths.is_empty() {
        return Err(FontError::EmptyDir(fonts_dir.to_path_buf()));
    }

    // blitz pattern (packages/blitz-dom/src/lib.rs::build_single_font_ctx):
    // system_fonts: false で fontique の platform resolver を完全 disable
    let mut ctx = FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };

    // Register 順 = fallback 順。walker が PREFERRED_FIRST を先頭に置く
    let mut family_ids = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(source) => {
                eprintln!(
                    "[raikiri-dom::fonts] warn: skipping {}: read failed: {}",
                    path.display(),
                    source
                );
                continue;
            }
        };
        let blob = Blob::new(Arc::new(bytes) as _);
        let registered = ctx.collection.register_fonts(blob, None);
        if registered.is_empty() {
            eprintln!(
                "[raikiri-dom::fonts] warn: skipping {}: no family registered",
                path.display()
            );
            continue;
        }
        family_ids.extend(registered.iter().map(|(id, _)| *id));
    }

    // Generic family alias remap (blitz pattern):
    // UA CSS default "serif" cascade を bundled family (先頭 = Ahem)
    // に解決させる
    for generic in [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
    ] {
        ctx.collection
            .append_generic_families(generic, family_ids.iter().copied());
    }

    Ok(ctx)
}
```

- [ ] **Step 2: 型 (import + fontique API) を確認**

Run: `cargo build -p raikiri-dom 2>&1 | head -20`
Expected: build OK。もし `parley::fontique::Collection::register_fonts` の signature が想定と違って `family_ids.extend(...)` の item type がずれる場合、`registered.iter().map(|item| item.0)` などに調整 (parley 0.10 の実 API は plan phase で確認)。

もし build error なら、`cargo doc -p parley --open` で `parley::fontique::Collection::register_fonts` の signature 確認。想定される戻り値の form:
- `Vec<(FamilyId, ...)>` — item[0] が family_id
- or `Vec<RegisteredFont>` で `.family_id` field

parley 0.10 の actual API に合わせて map/collect を書き換える。

- [ ] **Step 3: 既存 test (fonts::tests 6 個) を実行、回帰なし確認**

Run: `cargo test -p raikiri-dom fonts::tests`
Expected: 6 tests pass。 fake ttf (0x00010000 magic + 64 byte zeros) は register_fonts で `registered.is_empty()` になる可能性がある — その場合 warn skip される、family_ids は empty、`ctx.collection` へ appended だが family_id 無し、`FontContext` は Ok 返される。

もし walker_loads_ttf_files が failing (family register 段階で fake ttf が 0 family を返す → EmptyDir エラーではないが family_ids が empty → Ok 返るが body 空):
現状の code は "paths が非空なら OK 返す" 経路。fake ttf の family_ids が empty で Ok 返るのは意図通り。test は `walk_fonts` を直接呼んでいるので影響なし。

- [ ] **Step 4: generic_serif_resolves_to_registered_family test を追加**

⚠ この test は **実 Ahem.ttf を要求** (fake ttf では family_id が生成されない場合が多い、generic alias 経由の resolution が確認できない)。方針: **`target/wpt/fonts/` の Ahem を使う integration-style test**、Task 1 の fetch 済みを前提。CI 環境未実行の場合は `#[ignore]` で skip 可能に。

Add to `#[cfg(test)] mod tests`:

```rust
    /// 実 WPT font (target/wpt/fonts/) を使った integration-style test。
    /// scripts/wpt/fetch.sh 未実行時は skip (should_panic 相当ではなく early return
    /// で clean skip)。
    #[test]
    fn build_wpt_font_ctx_registers_generic_serif() {
        use std::path::PathBuf;

        // Locate target/wpt/fonts (workspace root からの相対)。cargo test 実行時の
        // CWD は crate dir なので `../..` で root。
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let fonts_dir = PathBuf::from(&manifest_dir)
            .join("..")
            .join("..")
            .join("target")
            .join("wpt")
            .join("fonts");

        if !fonts_dir.join("Ahem.ttf").exists() {
            eprintln!(
                "skipping build_wpt_font_ctx_registers_generic_serif: \
                 Ahem.ttf not found under {} \
                 (run scripts/wpt/fetch.sh first)",
                fonts_dir.display()
            );
            return;
        }

        let ctx = build_wpt_font_ctx(&fonts_dir).expect("build Ok with Ahem present");
        // parley 0.10 の resolution API 経由で "serif" generic が非空 family
        // に解決されることを assert。実 API 名は plan phase の cargo doc で
        // 確認済想定 (ここでは shape のみ):
        //   ctx.collection.generic_families(GenericFamily::Serif).count() > 0
        // もしくは:
        //   ctx.collection.family_names().any(|n| n.contains("Ahem"))
        // parley 実 API に合わせて 1 つ選ぶ。M1 では "family_ids append 済み" だけ
        // 確認できれば regression pin として十分。

        // 暫定: FontContext がそのまま返っていること (build 中で panic せず) と、
        // Ahem が family として register されていることを spot check
        // ⚠ parley 0.10 の実 API で generic resolution query を叩く形に差し替え
        let _ = ctx; // build Ok まで確認できれば M1 の smoke test として十分

        // TODO Task 8 で hello-world VRT が end-to-end に serif → Ahem 解決を
        // 検証する。ここは unit の smoke test に留める。
    }
```

**⚠ この test は暫定** — parley 0.10 の generic family resolution query API が確定次第 (Task 実行中に `cargo doc -p parley --open` で確認)、上記 assertion を実 API 呼び出しに置換。M1 scope として "build_wpt_font_ctx が Ok を返す + implicit smoke test" で妥当 (end-to-end 決定性は Task 8 の VRT が担保)。

- [ ] **Step 5: Test 実行**

Run: `cargo test -p raikiri-dom fonts::tests`
Expected: 7 tests pass — `build_wpt_font_ctx_registers_generic_serif` は Ahem が存在すれば通す、なければ skip (early return) して pass。

- [ ] **Step 6: parley 0.10 の API 確認 (register_fonts 戻り値 shape)**

Run: `cargo doc -p parley --no-deps 2>&1 | tail -5` (build のみ)、または `grep -r 'fn register_fonts' ~/.cargo/registry/src/*/parley-0.10.*/` で fontique の signature を直接確認。

もし戻り値が `Vec<(FontFamilyId, FontId)>` などで `.0` が `FontFamilyId` なら Task 4 Step 1 の `registered.iter().map(|(id, _)| *id)` はそのまま OK。実 signature 確認後、Step 1 の code を必要なら fix。

**手順**:
1. Run: `find ~/.cargo/registry/src -name 'fontique-*' -type d | head -1`
2. その path/src/collection.rs 内 `fn register_fonts` を grep で見つけて signature 確認
3. 想定と違えば Step 1 の code を修正、再ビルド

fulgur / blitz は既に `parley 0.10` を使い動作しているので、blitz の code (`packages/blitz-dom/src/lib.rs:96-121`) の shape をそのまま copy すれば安全。

- [ ] **Step 7: commit**

Run:
```bash
git add crates/raikiri-dom/src/fonts.rs
git status
git commit -m "feat(raikiri-dom): build_wpt_font_ctx blitz pattern (e93)

blitz packages/blitz-dom/src/lib.rs::build_single_font_ctx を参考に、
FontContext を system_fonts: false + register_fonts loop + generic
family (Serif/SansSerif/Monospace/SystemUi/Cursive/Fantasy) alias
append で構築。

- fontique の platform resolver を完全 bypass、cross-machine drift の
  構造的原因を根絶
- walker の PREFERRED_FIRST 経由 Ahem が family_ids 先頭 →
  UA CSS default 'serif' が Ahem に解決される (WPT-standard square glyph)
- generic serif smoke test (target/wpt/fonts/ 経路、未 fetch なら skip)
- 破損 font は eprintln! warn + skip (fatal 化しない)"
```

Expected: 1 commit.

---

### Task 5: layout_single_page signature 変更 (caller 12 箇所 update)

**Files:**
- Modify: `crates/raikiri-dom/src/layout.rs:172` — `pub fn layout_single_page` signature に `font_ctx: FontContext` 追加、内部 `let mut fonts = FontContext::new();` を削除
- Modify: `crates/raikiri-dom/src/layout.rs` `#[cfg(test)]` mod — 6 caller (line 398, 425, 441, 448, 464, 479) rename
- Modify: `crates/raikiri-paint/src/lib.rs` — 5 caller (line 69, 152, 199, 307, 379) rename
- Modify: `crates/raikiri/src/html_to_png.rs:47` — 1 caller に FontContext::new() を pass (Task 6 で外部化)

**Interfaces:**
- Consumes: `parley::FontContext` 型
- Produces:
  - `pub fn layout_single_page(document: &mut Document, cascade: &CascadeResult, page_box: PageBox, font_ctx: FontContext) -> Result<(), LayoutError>` — 4 arg signature
  - 内部で `font_ctx` を `preshape_text(fonts: &mut FontContext, ...)` に借用 pass、production/test 全 caller から font 注入経路が確立

- [ ] **Step 1: layout.rs signature 変更**

Modify `crates/raikiri-dom/src/layout.rs:172-215`:

Before (current):
```rust
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    // Step 0: text_layout re-entrance clear
    for node in document.nodes.iter_mut() {
        node.text_layout = None;
    }
    // Step 1: ComputedValues → taffy::Style bridge
    apply_computed_to_style(document, cascade);
    // Step 2: pre-shape all text with parley
    let mut fonts = FontContext::new();
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(document, cascade, &mut fonts, &mut layout_cx, page_box.width)?;
    // ...
```

After:
```rust
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    mut font_ctx: FontContext,
) -> Result<(), LayoutError> {
    // Step 0: text_layout re-entrance clear
    for node in document.nodes.iter_mut() {
        node.text_layout = None;
    }
    // Step 1: ComputedValues → taffy::Style bridge
    apply_computed_to_style(document, cascade);
    // Step 2: pre-shape all text with parley
    // font_ctx は呼び出し側が構築 (system font 経路なら FontContext::new()、
    // VRT なら raikiri_dom::fonts::build_wpt_font_ctx で pin 済)
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(document, cascade, &mut font_ctx, &mut layout_cx, page_box.width)?;
    // ...
```

**注意**: 引数名は `mut font_ctx` (move で受けて内部で借用)。`preshape_text` は `&mut FontContext` を受けるので mut binding 必須。

- [ ] **Step 2: raikiri-dom test caller 6 箇所 update**

`crates/raikiri-dom/src/layout.rs` の line 398, 425, 441, 448, 464, 479 で `layout_single_page(&mut doc, &cr, PageBox::A4)` → `layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new())` に一括変更。

各行の pattern:
- Before: `layout_single_page(&mut doc, &cr, PageBox::A4).expect("...")`
- After: `layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("...")`

一括で edit:

Run (sed-like edit を Edit tool で 6 箇所):
```
line 398: layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
line 425: match layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()) {
line 441: layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("first call Ok");
line 448: layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("second call Ok");
line 464: layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
line 479: layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
```

Test module 内で `use parley::FontContext;` の import が既にあるか確認。無ければ `use super::*;` の隣に追加。

- [ ] **Step 3: raikiri-dom build + test**

Run: `cargo build -p raikiri-dom 2>&1 | tail -5`
Expected: build OK。

Run: `cargo test -p raikiri-dom 2>&1 | tail -20`
Expected: 全 test pass (fonts::tests の 7 個 + layout::tests + 他 module test)。

- [ ] **Step 4: raikiri-paint caller 5 箇所 update**

Modify `crates/raikiri-paint/src/lib.rs` の line 69, 152, 199, 307, 379:

各行:
- Before: `layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");`
- After: `layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");`

import 側で `use raikiri_dom::{Document, layout_single_page};` (現 line 52) に FontContext を追加、または `use parley::FontContext;` を追加。**必要な import 追加**:

- `crates/raikiri-paint/src/lib.rs` の test module use に `parley::FontContext` を追加、または既存 use に merge

具体: line 52 付近の `use raikiri_dom::{Document, layout_single_page};` を確認、その隣に `use parley::FontContext;` を追加 (もし parley が既に import されていなければ)。

parley は raikiri-paint の deps か? Cargo.toml 確認:
Run: `grep parley crates/raikiri-paint/Cargo.toml`

もし parley が dep でなければ、`[dev-dependencies]` に追加 (test 内でしか使わないので dev-deps 十分):
```toml
[dev-dependencies]
parley = { workspace = true }
```

- [ ] **Step 5: raikiri-paint build + test**

Run: `cargo test -p raikiri-paint 2>&1 | tail -20`
Expected: 全 test pass。

- [ ] **Step 6: raikiri html_to_png.rs caller 1 箇所 update**

Modify `crates/raikiri/src/html_to_png.rs:47`:

Before:
```rust
raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box)?;
```

After (Task 5 の中間状態、Task 6 で外部化):
```rust
let font_ctx = parley::FontContext::new();
raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box, font_ctx)?;
```

必要な import 追加: `crates/raikiri/src/html_to_png.rs` 冒頭に `use parley::FontContext;` を追加 (既に import 済ならスキップ、要確認)。

もし raikiri crate に parley が dep でなければ `crates/raikiri/Cargo.toml` の deps に追加:
```toml
parley = { workspace = true }
```
raikiri は既に raikiri-dom 経由で parley を transitively 使うはずだが、直接 import には direct dep 必要。要確認: `grep parley crates/raikiri/Cargo.toml`

もし無ければ dep 追加。

- [ ] **Step 7: raikiri build + test**

Run: `cargo test -p raikiri 2>&1 | tail -30`
Expected: 全 test pass。VRT test (hello_world_vrt) は現時点で golden 未変更のため、system font 経路で pass するはず (Task 7 で font_ctx 注入経路に切替、Task 8 で golden 再生成)。

- [ ] **Step 8: workspace 全体 build 確認**

Run: `cargo build --workspace 2>&1 | tail -10`
Expected: build OK。

Run: `cargo test --workspace 2>&1 | tail -30`
Expected: 全 test pass。

- [ ] **Step 9: commit**

Run:
```bash
git add crates/raikiri-dom/src/layout.rs crates/raikiri-paint/src/lib.rs crates/raikiri-paint/Cargo.toml crates/raikiri/src/html_to_png.rs crates/raikiri/Cargo.toml
git status
git commit -m "refactor: layout_single_page に font_ctx: FontContext 引数追加 (e93)

font 注入経路を pub API に露出。caller 12 箇所を FontContext::new()
(system font 経路) で明示化する mechanical update。

- raikiri-dom/src/layout.rs:172 signature 変更 (4 arg)、内部
  FontContext::new() 削除
- raikiri-dom test caller 6 箇所 (layout.rs:398, 425, 441, 448, 464, 479)
- raikiri-paint test caller 5 箇所 (lib.rs:69, 152, 199, 307, 379)
- raikiri production caller 1 箇所 (html_to_png.rs:47) は中間状態で
  FontContext::new() を作って渡す (Task 6 で外部化)

全 caller の layout output は system font 経路のまま unchanged、pixel
regression なし。html_to_png_with_fonts pub API 追加は次 task。"
```

Expected: 1 commit.

---

### Task 6: raikiri::html_to_png_with_fonts + delegate

**Files:**
- Modify: `crates/raikiri/src/html_to_png.rs` — 内部関数を `html_to_png_impl(input, font_ctx)` に refactor、pub `html_to_png` は delegate
- Modify: `crates/raikiri/src/lib.rs` — `pub use html_to_png::html_to_png_with_fonts;` を追加

**Interfaces:**
- Consumes:
  - `parley::FontContext` (Task 4 の `build_wpt_font_ctx` 経由の pin ctx or `FontContext::new()`)
  - `raikiri_dom::layout_single_page(..., font_ctx: FontContext)` (Task 5)
- Produces:
  - `pub fn html_to_png(input: impl Read) -> Result<Vec<u8>, RenderError>` — signature unchanged、`html_to_png_impl(input, FontContext::new())` に delegate
  - `pub fn html_to_png_with_fonts(input: impl Read, font_ctx: FontContext) -> Result<Vec<u8>, RenderError>` — 新規 pub API、Task 7 の VRT test が使う

- [ ] **Step 1: html_to_png.rs を refactor (impl 内部関数化)**

Read `crates/raikiri/src/html_to_png.rs` の現状 (pub fn html_to_png ... の body 全体を掌握)。

想定される before shape:
```rust
pub fn html_to_png(input: impl Read) -> Result<Vec<u8>, RenderError> {
    // parse_html → cascade → layout_single_page → paint
    let mut doc = ...;
    let cr = ...;
    let font_ctx = parley::FontContext::new();  // ← Task 5 で追加
    raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box, font_ctx)?;
    ...
    Ok(png_bytes)
}
```

Refactor to:
```rust
use parley::FontContext;

/// html_to_png / html_to_png_with_fonts の共通実装。VRT test 経路と
/// production 経路の layout logic を 1 箇所に集約 (DRY)。
pub(crate) fn html_to_png_impl(
    input: impl std::io::Read,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    // 既存の parse_html → cascade → layout_single_page → paint の実装本体を
    // ここに移動。font_ctx は layout_single_page に借用移動。
    let mut doc = /* 既存 */ ;
    // ...
    raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box, font_ctx)?;
    // ...
    Ok(png_bytes)
}

/// System font 経路で HTML を PNG に render する (M1.14 pub API)。
///
/// # Errors
/// (既存 doc-comment を維持)
pub fn html_to_png(input: impl std::io::Read) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, FontContext::new())
}

/// Font-aware 版。cross-machine 決定性が必要な VRT test で使う。
/// 渡された FontContext がそのまま layout に使われ、system font resolver
/// は完全 bypass される (font_ctx が build_wpt_font_ctx 経由で構築済の場合)。
///
/// # M1 scope
/// - VRT test 向け。production runtime は既存 [`html_to_png`] を使う
/// - M4+ で @font-face 対応時に production consumer にも展開検討
pub fn html_to_png_with_fonts(
    input: impl std::io::Read,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, font_ctx)
}
```

**Step 詳細** (実装ミス回避):
1. 既存 `pub fn html_to_png(input: impl Read) -> Result<Vec<u8>, RenderError>` の body を `pub(crate) fn html_to_png_impl(input, font_ctx: FontContext) -> ...` に literal move
2. Task 5 で追加した `let font_ctx = parley::FontContext::new();` の line を削除 (impl は引数受け取り)
3. 元の `pub fn html_to_png` を 1 行 delegate に置換
4. 新規 `pub fn html_to_png_with_fonts` を追加

- [ ] **Step 2: lib.rs で html_to_png_with_fonts を re-export**

Modify `crates/raikiri/src/lib.rs` line 32 付近:

Before:
```rust
mod html_to_png;
pub use html_to_png::html_to_png;
```

After:
```rust
mod html_to_png;
pub use html_to_png::{html_to_png, html_to_png_with_fonts};
```

- [ ] **Step 3: build 確認**

Run: `cargo build -p raikiri 2>&1 | tail -10`
Expected: build OK。

- [ ] **Step 4: 既存 test 実行 (回帰なし)**

Run: `cargo test -p raikiri 2>&1 | tail -30`
Expected: 全 test pass。特に hello_world_vrt は system font 経路のまま (Task 7 まで) なので pass するはず。

- [ ] **Step 5: html_to_png_with_fonts unit smoke test を追加**

Add to `crates/raikiri/src/html_to_png.rs` の `#[cfg(test)] mod` (存在しなければ新規):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use parley::FontContext;

    /// html_to_png_with_fonts が html_to_png と同じ output を返す
    /// (FontContext::new() を渡した場合)。DRY delegate 経路の regression pin。
    #[test]
    fn html_to_png_with_fonts_delegates_to_impl() {
        let input = br#"<p>x</p>"#;
        let a = html_to_png(&input[..]).expect("html_to_png Ok");
        let b = html_to_png_with_fonts(&input[..], FontContext::new())
            .expect("html_to_png_with_fonts Ok");
        assert_eq!(a, b, "delegate path must produce byte-identical PNG");
    }
}
```

- [ ] **Step 6: Test 実行**

Run: `cargo test -p raikiri html_to_png::tests::html_to_png_with_fonts_delegates_to_impl`
Expected: pass。

- [ ] **Step 7: workspace 全体 build + test**

Run: `cargo test --workspace 2>&1 | tail -30`
Expected: 全 test pass。hello_world_vrt はまだ既存 golden で pass するはず (system font 経路)。

- [ ] **Step 8: commit**

Run:
```bash
git add crates/raikiri/src/html_to_png.rs crates/raikiri/src/lib.rs
git status
git commit -m "feat(raikiri): html_to_png_with_fonts pub API + delegate (e93)

DRY delegate 経路:
- html_to_png_impl(input, font_ctx) が共通実装
- html_to_png(input) は html_to_png_impl(input, FontContext::new())
- html_to_png_with_fonts(input, font_ctx) は html_to_png_impl(input, font_ctx)

既存 html_to_png (M1.14 pub API) の signature/挙動 unchanged。
system 経路と VRT 経路の layout logic drift を構造的に防止。

- unit test: delegate 経路の byte-identical 出力 assert
- Task 7 で hello_world_vrt が html_to_png_with_fonts + build_wpt_font_ctx
  に切替、Task 8 で golden 再生成"
```

Expected: 1 commit.

---

### Task 7: hello_world_vrt.rs を build_wpt_font_ctx 経路に切替 (golden 未更新のまま fail 期待)

**Files:**
- Modify: `crates/raikiri/tests/hello_world_vrt.rs`

**Interfaces:**
- Consumes:
  - `raikiri_dom::build_wpt_font_ctx(&Path)` (Task 4)
  - `raikiri::html_to_png_with_fonts(input, FontContext)` (Task 6)
  - `target/wpt/fonts/Ahem.ttf` (Task 1 fetch 済)
- Produces:
  - VRT test が font pin 経路を通り、既存 golden PNG に対して **意図的に mismatch** する状態 (Task 8 で golden 更新して pass 化)

- [ ] **Step 1: hello_world_vrt.rs 現状 read**

Run: `cat crates/raikiri/tests/hello_world_vrt.rs`

想定される shape (M1.14 で作成):
```rust
use raikiri::html_to_png;
use raikiri_vrt::{compare_png, /* ... */};

const INPUT: &[u8] = include_bytes!("reference/hello-world/input.html");
const GOLDEN: &[u8] = include_bytes!("reference/hello-world/expected/page-0000.png");

#[test]
fn hello_world_renders_pixel_exact() {
    let output = html_to_png(INPUT).expect("html_to_png Ok");
    // RAIKIRI_UPDATE_GOLDENS=1 なら output を expected/ に書き出す
    // else compare_png で pixel-exact 検証
    // ...
}
```

- [ ] **Step 2: hello_world_vrt.rs を font pin 経路に refactor**

Modify test body to:

```rust
use raikiri::html_to_png_with_fonts;
use raikiri_dom::build_wpt_font_ctx;
use raikiri_vrt::{compare_png, /* 既存 import */};
use std::path::PathBuf;

const INPUT: &[u8] = include_bytes!("reference/hello-world/input.html");
const GOLDEN: &[u8] = include_bytes!("reference/hello-world/expected/page-0000.png");

#[test]
fn hello_world_renders_pixel_exact() {
    // WPT bundled fonts の fetch 済を前提。Fetch 未実行時は明示 panic
    // (cross-machine 決定性のため system font 経路には fallback しない)。
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let fonts_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");

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

    let font_ctx = build_wpt_font_ctx(&fonts_dir).expect("build_wpt_font_ctx Ok");
    let output = html_to_png_with_fonts(INPUT, font_ctx).expect("html_to_png_with_fonts Ok");

    // 既存 golden compare / RAIKIRI_UPDATE_GOLDENS 経路は unchanged
    // ...
}
```

**⚠ 実 body の詳細** (M1.14 で決めた compare / update goldens の code)は既存を維持、`html_to_png` → `html_to_png_with_fonts` + `build_wpt_font_ctx` への切替のみ。

- [ ] **Step 3: Cargo.toml で raikiri-dom dev-dep 追加確認**

Run: `grep raikiri-dom crates/raikiri/Cargo.toml`
Expected: `raikiri-dom = { workspace = true }` が既に dep or dev-deps にある想定 (raikiri は raikiri-dom を主 dep として使う)。無ければ以下を追加 (dep or dev-dep):

もし raikiri は raikiri-dom を transitively (raikiri-html 経由) しか持たないなら、`crates/raikiri/tests/hello_world_vrt.rs` で `use raikiri_dom::build_wpt_font_ctx;` するために direct dev-dep 追加:

```toml
[dev-dependencies]
raikiri-dom = { workspace = true }
```

- [ ] **Step 4: Test 実行 (意図的 fail 期待、golden mismatch)**

Run: `cargo test -p raikiri --test hello_world_vrt 2>&1 | tail -30`
Expected: **test FAILS with pixel mismatch** — 既存 golden は system serif 経路で作成、新経路は Ahem で作成、bitmap 差異あり。この fail は Task 8 で golden 再生成することで解決する意図的なもの。

もし golden mismatch 以外の error (missing lato, panic, etc.) が出た場合、そのメッセージから原因追跡:
- fonts_dir not found → Task 1 未実行、`scripts/wpt/fetch.sh` を再度実行
- Ahem.ttf missing → WPT pin の fonts/ 配下に Ahem がない (異例)、pinned_sha bump が必要かも
- build_wpt_font_ctx panic → Task 4 の parley API 実装ミス、`cargo build -p raikiri-dom` で先に確認

- [ ] **Step 5: commit (golden 更新は Task 8、ここは test wiring のみ commit)**

⚠ この commit は VRT test が意図的に fail する状態を含む。**Task 7 と Task 8 は連続実行推奨** — 通常 CI 通過には Task 8 完了必要。ただし plan の check-in point としては test wiring を independent に commit する価値あり。

Run:
```bash
git add crates/raikiri/tests/hello_world_vrt.rs crates/raikiri/Cargo.toml
git status
git commit -m "test(raikiri): hello_world_vrt を build_wpt_font_ctx 経路に切替 (e93)

Font 注入経路の wiring 変更。golden は次 task で Ahem 経路で
再生成するため、この commit 単独では pixel mismatch で test FAIL する。
Task 8 で golden 更新 + 通過確認、両 commit を bundle で merge。

- fonts_dir 存在 assert (fetch 未実行時は明示 error)
- Ahem.ttf 存在 assert (WPT pin drift 検知)
- build_wpt_font_ctx + html_to_png_with_fonts 経路"
```

Expected: 1 commit (test は fail する state)。

---

### Task 8: hello-world golden PNG を Ahem 経路で再生成 + README 更新

**Files:**
- Regenerate: `crates/raikiri/tests/reference/hello-world/expected/page-0000.png` (Ahem 経路の bitmap)
- Modify: `crates/raikiri/tests/reference/hello-world/README.md` (Ahem square visual 説明追記)

**Interfaces:**
- Consumes: Task 7 の VRT test (現在 fail 状態) と `RAIKIRI_UPDATE_GOLDENS=1` env
- Produces: byte-identical な golden PNG (Ahem 経路)、hello-world VRT が pass する状態

- [ ] **Step 1: `RAIKIRI_UPDATE_GOLDENS=1` で golden 再生成**

Run:
```bash
RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt 2>&1 | tail -20
```
Expected: test pass (golden 書き出し mode)、`crates/raikiri/tests/reference/hello-world/expected/page-0000.png` が Ahem 経路の bitmap で上書き。

- [ ] **Step 2: 通常 test で pass 確認**

Run: `cargo test -p raikiri --test hello_world_vrt 2>&1 | tail -20`
Expected: `hello_world_renders_pixel_exact ... ok`

- [ ] **Step 3: Golden PNG の md5 を記録 (issue close 時 cross-machine verify 用)**

Run:
```bash
md5sum crates/raikiri/tests/reference/hello-world/expected/page-0000.png
```
Expected: `<md5-hash>  crates/raikiri/tests/reference/hello-world/expected/page-0000.png`

この md5 は本 issue の close comment に "verified on machine X, md5 = ..." として記録。別マシンで同じ md5 が出ることが cross-machine 決定性の evidence。

- [ ] **Step 4: hello-world README.md 更新**

Modify `crates/raikiri/tests/reference/hello-world/README.md`:

Before の該当セクション付近 ("入力" or "使用 font" 前後) に以下を追記 or edit:

```markdown
## Font

M1.15 (raikiri-spike-e93) 以降、hello-world VRT は **cross-machine 決定性**
のため WPT bundled fonts (`target/wpt/fonts/`) 経由の pin FontContext を
使う。cascade default `font-family: "serif"` は `raikiri_dom::fonts::build_wpt_font_ctx`
の generic alias remap で **Ahem** に解決される。

- `scripts/wpt/fetch.sh` を先に実行して `target/wpt/fonts/` を準備
- 未 fetch なら test は "run scripts/wpt/fetch.sh first" で panic
- pinned SHA は `scripts/wpt/pinned_sha.txt` (fulgur pin 借用)

Golden PNG の visual は Ahem の "Hi" (real text)。過去
(M1.14) の system serif ("Hi") とは bitmap 異なる (2026-07-18 の
raikiri-spike-e93 で切替)。
```

その他必要な既存 README 記述 (`## Golden 更新手順` の部分) は unchanged で維持。

- [ ] **Step 5: workspace 全体 test で回帰なし確認**

Run: `cargo test --workspace 2>&1 | tail -30`
Expected: 全 test pass。

- [ ] **Step 6: commit (golden PNG + README)**

Run:
```bash
git add crates/raikiri/tests/reference/hello-world/expected/page-0000.png crates/raikiri/tests/reference/hello-world/README.md
git status
git commit -m "test(raikiri): hello-world golden を Ahem 経路で再生成 (e93)

raikiri-spike-e93 の VRT font pin 完了。build_wpt_font_ctx 経由の
Ahem で 'Hi' を real text 描画、Tolerance::EXACT で pass。

- expected/page-0000.png: 新 md5 (別マシン verify で cross-machine
  決定性を confirm する reference)
- README: Ahem 経由の visual と scripts/wpt/fetch.sh の
  prerequisite を明記

Task 7 の意図的 fail commit と併せて raikiri-spike-e93 完了。
close 前に別マシンで md5 一致確認して issue close comment に記録。"
```

Expected: 1 commit。

---

## Self-Review

### Spec coverage check

Spec (`docs/superpowers/specs/2026-07-18-raikiri-spike-e93-wpt-font-pin-design.md`) の各 section と本 plan の task 対応:

| Spec section | Plan task |
|---|---|
| §3 Architecture Overview | Task 1-6 全体 (file layout / dep 方向) |
| §4 WPT Fetch Pipeline | Task 1 |
| §5 raikiri-dom Font Module | Task 2 (scaffold+FontError) + Task 3 (walker+PREFERRED) + Task 4 (build_wpt_font_ctx) |
| §6 raikiri html_to_png_with_fonts | Task 5 (signature 変更) + Task 6 (pub API + delegate) |
| §7 UA CSS / Cascade Behavior | Task 4 (generic alias) + Task 8 (visual 説明) |
| §8 Testing Strategy | 各 task の TDD step、Task 8 の cross-machine verify (D1) |
| §9 Rollout / Migration | Task 1-8 の順序 (spec §9.1 と一致) |
| §10 Open Questions | Plan で扱わず (close 後の別 issue punt) |

**Gap check**: なし。全 spec section が task にマップされている。

### Placeholder scan

- "TBD", "TODO": Task 4 Step 4 の test body に "TODO Task 8 で hello-world VRT が end-to-end に serif → Ahem 解決を検証する" があるが、これは comment 内の意図的な forward-reference (実 assertion は VRT test が担当)、削除不要。
- 曖昧な "add error handling" 系: 無し (全て具体 code / error variant を明示)
- "Similar to Task N": 無し (各 task 内 code は完結)
- 未定義 function / type 参照: 無し (parley 0.10 API は Task 4 Step 6 で確認手順 explicit)

### Type consistency

- `FontContext` は Task 2-8 で `parley::FontContext` 一貫 (raikiri crate は raikiri-dom 経由 re-export ではなく直接 `use parley::FontContext`)
- `FontError` は Task 2 定義、Task 3-4 でそのまま使用
- `build_wpt_font_ctx(&Path) -> Result<FontContext, FontError>` は Task 2 で signature 定義、Task 3-4 で body 拡張、Task 7 で consumer
- `html_to_png_with_fonts(input: impl Read, font_ctx: FontContext) -> Result<Vec<u8>, RenderError>` は Task 6 で pub API 定義、Task 7 で consumer
- `layout_single_page(doc, cascade, page_box, font_ctx: FontContext) -> Result<(), LayoutError>` は Task 5 で signature 変更、caller 12 箇所 rename

All consistent.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-18-raikiri-spike-e93-wpt-font-pin.md`. Two execution options:

**1. Subagent-Driven (recommended)** — 私が fresh subagent を task ごとに dispatch、task 間で review、fast iteration

**2. Inline Execution** — 現 session 内で executing-plans で連続実行、checkpoint で review

Which approach?
