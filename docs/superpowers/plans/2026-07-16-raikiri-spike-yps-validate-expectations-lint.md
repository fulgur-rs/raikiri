# validate-expectations lint Implementation Plan (raikiri-spike-yps)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** raikiri-wpt に spec §12.10 の 4 種検出 (Malformed / Duplicate / Conflicting / Expired) を持つ `lint` モジュールと `validate-expectations` bin を実装する。

**Architecture:** 新規 `raikiri_wpt::lint` モジュールが検出ロジックを持ち、`validate-expectations` bin は薄い wrapper。既存の `raikiri_wpt::expectations` パーサは変更しない (公開 API 維持、breaking change 回避)。日付は `time::Date` で扱う。`now` を注入可能にして unit test を決定論化する。

**Tech Stack:** Rust 2024 edition, `time` crate (parsing + macros features), `insta` (既存 dev-dep, snapshot), `tempfile` (新規 dev-dep, fixture directory), `assert_cmd` (新規 dev-dep, bin テスト)。

## Global Constraints

- MSRV = 1.89.0 (workspace `rust-version`)
- `unsafe_code = "deny"` (workspace lint)
- `missing_docs = "warn"` — 公開 items には `///` ドキュメントを書く
- 既存 `crates/raikiri-wpt/src/expectations.rs` の公開 API を変更しない (追加は OK、既存 signature 変更は不可)
- `chrono` / `jiff` は使わない (workspace 統一で `time`)
- CI 配線は yps の scope 外 (`raikiri-spike-m1.12` の担当)
- HTTP-based issue-closed check は yps の scope 外 (別 issue に切り出す)
- Filter overlap 解析は yps の scope 外 (別 issue に切り出す)

---

## File Structure

**Create:**
- `crates/raikiri-wpt/src/lint.rs` — 4 種検出ロジック本体 (`LintReport`, `LintIssue`, `Category`, `run`)

**Modify:**
- `Cargo.toml` (workspace) — `time`, `tempfile`, `assert_cmd` を workspace.dependencies に追加
- `crates/raikiri-wpt/Cargo.toml` — `time` を `[dependencies]`、`tempfile` / `assert_cmd` を `[dev-dependencies]` に追加
- `crates/raikiri-wpt/src/lib.rs` — `pub mod lint;` を追加
- `crates/raikiri-wpt/src/bin/validate-expectations.rs` — 現状 10 行の stub を lint::run 呼び出し + exit code 設定に置換

**Test:**
- unit tests は `crates/raikiri-wpt/src/lint.rs` 内 `#[cfg(test)] mod tests`
- integration test は `crates/raikiri-wpt/tests/lint_integration.rs` (tempfile で fixture dir を stage)
- bin test は `crates/raikiri-wpt/tests/validate_expectations_bin.rs` (assert_cmd)

---

## Task 1: Scaffold — deps + module + types + smoke test

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/raikiri-wpt/Cargo.toml`
- Create: `crates/raikiri-wpt/src/lint.rs`
- Modify: `crates/raikiri-wpt/src/lib.rs`

**Interfaces:**
- Produces:
  - `raikiri_wpt::lint::LintReport { issues: Vec<LintIssue> }`
  - `LintReport::has_failures(&self) -> bool` — `true` iff any `Category` other than `Expired`
  - `LintReport::is_empty(&self) -> bool`
  - `raikiri_wpt::lint::LintIssue { category: Category, file: String, line_no: Option<usize>, message: String }`
  - `raikiri_wpt::lint::Category` (non-exhaustive enum: `Malformed`, `Duplicate`, `Conflicting`, `Expired`)
  - `Category::is_failure(&self) -> bool` — `true` for all except `Expired`
  - `raikiri_wpt::lint::run(dir: &Path, now: time::Date) -> LintReport` — 現時点では空 `LintReport` を返す

- [ ] **Step 1: workspace deps を追加**

`Cargo.toml` (workspace) の `[workspace.dependencies]` に 3 crate を追加。既存 utility crates block (`smol_str` から `url` の直後) に配置:

```toml
# ── Date/time (validate-expectations lint, raikiri-spike-yps) ──────────
time = { version = "0.3", default-features = false, features = ["std", "parsing", "macros"] }
```

`# ── Dev-only: VRT rasterizer / snapshot / oracle ───` block の直後に:

```toml
# ── Dev-only: lint fixtures + bin tests (raikiri-spike-yps) ────────────
tempfile   = "3"
assert_cmd = "2"
```

- [ ] **Step 2: raikiri-wpt Cargo.toml に dep を追加**

`crates/raikiri-wpt/Cargo.toml` の `[dependencies]` に:

```toml
time = { workspace = true }
```

`[dev-dependencies]` の末尾に:

```toml
tempfile     = { workspace = true }
assert_cmd   = { workspace = true }
```

- [ ] **Step 3: `lint.rs` の型定義と `run` stub を作成**

`crates/raikiri-wpt/src/lint.rs` を新規作成:

```rust
//! Lint checks for `expectations/` files (spec §12.10).
//!
//! Detects 4 categories of issues:
//! - [`Category::Malformed`] — shape/enum/encoding errors (via
//!   [`crate::expectations::ExpectError`] surfaced through this module)
//! - [`Category::Duplicate`] — repeat rows within a single file
//! - [`Category::Conflicting`] — same `test_id` in multiple expectations files
//! - [`Category::Expired`] — quarantine entries older than 90 days
//!
//! Consumed by the `validate-expectations` bin. Malformed / Duplicate /
//! Conflicting cause CI to fail; Expired is warning-only.

use std::path::Path;

use time::Date;

/// Aggregate lint result for a single `expectations/` directory scan.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LintReport {
    /// All issues detected across the 4 categories.
    pub issues: Vec<LintIssue>,
}

impl LintReport {
    /// True if any issue would block CI (all categories except [`Category::Expired`]).
    pub fn has_failures(&self) -> bool {
        self.issues.iter().any(|i| i.category.is_failure())
    }

    /// True if no issues were detected.
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

/// A single lint finding.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LintIssue {
    /// Which of the 4 categories this issue belongs to.
    pub category: Category,
    /// File the issue anchors to (logical name or full path).
    pub file: String,
    /// 1-based line number, when the issue anchors to a specific line.
    pub line_no: Option<usize>,
    /// Human-readable description of the issue.
    pub message: String,
}

/// The 4 spec §12.10 detection categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Category {
    /// Wrong shape / unknown enum / bad encoding.
    Malformed,
    /// Same row appears twice in a single file.
    Duplicate,
    /// Same test id appears in multiple expectations files.
    Conflicting,
    /// Quarantine entry older than 90 days (warning only).
    Expired,
}

impl Category {
    /// True for all categories except [`Category::Expired`].
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Expired)
    }
}

/// Scan an `expectations/` directory and return a [`LintReport`].
///
/// `now` is injected (rather than sourced from the system clock) so
/// [`Category::Expired`] detection is deterministic in tests.
pub fn run(_dir: &Path, _now: Date) -> LintReport {
    LintReport::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn header_only_dir() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        write(dir.path(), "tracked-wpt.txt", "# header\n");
        write(dir.path(), "known-issues.txt", "# header\n");
        write(dir.path(), "raikiri-baseline.txt", "# header\n");
        write(dir.path(), "quarantine.txt", "# header\n");
        write(dir.path(), "deprecated.txt", "# header\n");
        dir
    }

    #[test]
    fn empty_expectations_dir_yields_empty_report() {
        let dir = header_only_dir();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.is_empty(), "expected empty, got {:?}", report.issues);
        assert!(!report.has_failures());
    }

    #[test]
    fn category_is_failure_maps_expired_to_false() {
        assert!(Category::Malformed.is_failure());
        assert!(Category::Duplicate.is_failure());
        assert!(Category::Conflicting.is_failure());
        assert!(!Category::Expired.is_failure());
    }
}
```

- [ ] **Step 4: `lib.rs` にモジュール登録**

`crates/raikiri-wpt/src/lib.rs` の既存モジュール宣言に隣接して `pub mod lint;` を追加。

- [ ] **Step 5: cargo check + test を走らせて green を確認**

```bash
cargo check -p raikiri-wpt
cargo test -p raikiri-wpt --lib lint::
```

Expected: 2 tests passed (`empty_expectations_dir_yields_empty_report`, `category_is_failure_maps_expired_to_false`).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/raikiri-wpt/Cargo.toml crates/raikiri-wpt/src/lib.rs crates/raikiri-wpt/src/lint.rs
git commit -m "feat(raikiri-wpt): scaffold lint module for validate-expectations (raikiri-spike-yps)"
```

---

## Task 2: Malformed detection

**Files:**
- Modify: `crates/raikiri-wpt/src/lint.rs`

**Interfaces:**
- Consumes:
  - Existing `crate::expectations::{TrackedWpt, KnownIssues, Baseline, Quarantine, Deprecated}::load(path)` (each returns `Result<Self, ExpectError>`)
  - `crate::expectations::ExpectError` variants: `Io`, `MalformedLine { file, line_no, reason }`, `UnknownEnum { file, line_no, field, value }`
- Produces:
  - Private `struct Loaded` inside `lint.rs` holding `Option<T>` per file + raw content strings for later duplicate re-scan
  - Private `fn load_all(dir: &Path) -> Loaded` that also accumulates `LintIssue { category: Malformed }` for each parse failure
  - `pub fn run` now returns those Malformed issues instead of `LintReport::default()`

**Design note:** each file is loaded individually so one bad file does not hide errors in other files. Each parser is fail-fast within a single file (only the first bad line surfaces), which is acceptable for M3 baseline. Multi-error per file is a possible future enhancement.

- [ ] **Step 1: Failing tests — malformed baseline and quarantine**

`lint.rs` の `mod tests` に追加:

```rust
    #[test]
    fn malformed_quarantine_row_count_becomes_lint_issue() {
        let dir = header_only_dir();
        // 3 columns instead of 8
        write(dir.path(), "quarantine.txt", "css/foo | linux | x86_64\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        let issue = report
            .issues
            .iter()
            .find(|i| i.category == Category::Malformed)
            .expect("malformed issue expected");
        assert!(issue.file.ends_with("quarantine.txt"));
        assert_eq!(issue.line_no, Some(1));
        assert!(issue.message.contains("expected 8"), "got: {}", issue.message);
    }

    #[test]
    fn malformed_unknown_platform_becomes_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let issue = report
            .issues
            .iter()
            .find(|i| i.category == Category::Malformed)
            .expect("malformed issue expected");
        assert!(issue.message.contains("platform"), "got: {}", issue.message);
        assert!(issue.message.contains("plan9"), "got: {}", issue.message);
    }

    #[test]
    fn malformed_known_issues_missing_reason_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "known-issues.txt", "css/foo | \n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|i| i.category == Category::Malformed)
                .count(),
            1
        );
    }
```

- [ ] **Step 2: 走らせて赤を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 3 new tests fail (all report empty since `run` still stub).

- [ ] **Step 3: `load_all` と Malformed 検出を実装**

`lint.rs` の `run` 上部に private helpers を追加、`run` 本体を書き換え。既存の型定義はそのまま:

```rust
use crate::expectations::{Baseline, Deprecated, ExpectError, KnownIssues, Quarantine, TrackedWpt};

/// Successfully-loaded content per file, alongside raw text used later for
/// duplicate re-scan. Fields are `None` when the file failed to parse (in
/// which case a [`Category::Malformed`] issue was recorded in `issues`).
#[derive(Default)]
struct Loaded {
    tracked: Option<TrackedWpt>,
    known_issues: Option<KnownIssues>,
    baseline: Option<Baseline>,
    quarantine: Option<Quarantine>,
    deprecated: Option<Deprecated>,
    baseline_raw: Option<String>,
    quarantine_raw: Option<String>,
    deprecated_raw: Option<String>,
    issues: Vec<LintIssue>,
}

fn expect_error_to_issue(err: ExpectError, path_display: String) -> LintIssue {
    match err {
        ExpectError::Io(e) => LintIssue {
            category: Category::Malformed,
            file: path_display,
            line_no: None,
            message: format!("I/O error: {e}"),
        },
        ExpectError::MalformedLine { file, line_no, reason } => LintIssue {
            category: Category::Malformed,
            file,
            line_no: Some(line_no),
            message: format!("malformed line ({reason})"),
        },
        ExpectError::UnknownEnum { file, line_no, field, value } => LintIssue {
            category: Category::Malformed,
            file,
            line_no: Some(line_no),
            message: format!("unknown {field} value {value:?}"),
        },
    }
}

fn load_all(dir: &Path) -> Loaded {
    let mut out = Loaded::default();

    let tracked_path = dir.join("tracked-wpt.txt");
    match TrackedWpt::load(&tracked_path) {
        Ok(v) => out.tracked = Some(v),
        Err(e) => out.issues.push(expect_error_to_issue(e, tracked_path.display().to_string())),
    }

    let known_path = dir.join("known-issues.txt");
    match KnownIssues::load(&known_path) {
        Ok(v) => out.known_issues = Some(v),
        Err(e) => out.issues.push(expect_error_to_issue(e, known_path.display().to_string())),
    }

    let baseline_path = dir.join("raikiri-baseline.txt");
    match std::fs::read_to_string(&baseline_path) {
        Ok(raw) => {
            match Baseline::parse(&raw, &baseline_path.display().to_string()) {
                Ok(v) => out.baseline = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, baseline_path.display().to_string())),
            }
            out.baseline_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), baseline_path.display().to_string())),
    }

    let quarantine_path = dir.join("quarantine.txt");
    match std::fs::read_to_string(&quarantine_path) {
        Ok(raw) => {
            match Quarantine::parse(&raw, &quarantine_path.display().to_string()) {
                Ok(v) => out.quarantine = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, quarantine_path.display().to_string())),
            }
            out.quarantine_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), quarantine_path.display().to_string())),
    }

    let deprecated_path = dir.join("deprecated.txt");
    match std::fs::read_to_string(&deprecated_path) {
        Ok(raw) => {
            match Deprecated::parse(&raw, &deprecated_path.display().to_string()) {
                Ok(v) => out.deprecated = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, deprecated_path.display().to_string())),
            }
            out.deprecated_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), deprecated_path.display().to_string())),
    }

    out
}

pub fn run(dir: &Path, _now: Date) -> LintReport {
    let loaded = load_all(dir);
    LintReport { issues: loaded.issues }
}
```

- [ ] **Step 4: テストを走らせて緑を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 5 tests passed (2 previous + 3 new).

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-wpt/src/lint.rs
git commit -m "feat(raikiri-wpt/lint): detect Malformed via expectation parser errors (raikiri-spike-yps)"
```

---

## Task 3: Duplicate detection

**Files:**
- Modify: `crates/raikiri-wpt/src/lint.rs`

**Interfaces:**
- Consumes: `Loaded::baseline_raw`, `Loaded::deprecated_raw`, `Loaded::quarantine_raw` (from Task 2)
- Produces:
  - Private `fn detect_duplicates(loaded: &Loaded) -> Vec<LintIssue>` — line-level re-scan
  - `run` now extends its issues with duplicate detection output

**Design note:** the existing parsers use `HashSet` for `Baseline` and `Deprecated`, which silently dedupes. Line-level re-scan is the pragmatic path — it does not require changing the parser API. Non-comment, non-blank lines are considered.

- [ ] **Step 1: Failing tests — duplicates in each of 3 files**

`mod tests` に追加:

```rust
    #[test]
    fn duplicate_baseline_test_id_becomes_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "raikiri-baseline.txt",
            "css/foo/bar-001\ncss/foo/bar-001\ncss/foo/baz-002\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert!(dup[0].file.ends_with("raikiri-baseline.txt"));
        assert_eq!(dup[0].line_no, Some(2));
        assert!(dup[0].message.contains("css/foo/bar-001"), "got: {}", dup[0].message);
        assert!(dup[0].message.contains("line 1"), "got: {}", dup[0].message);
    }

    #[test]
    fn duplicate_deprecated_test_id_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "deprecated.txt", "css/x\ncss/y\ncss/x\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].line_no, Some(3));
    }

    #[test]
    fn duplicate_quarantine_tuple_becomes_lint_issue() {
        let dir = header_only_dir();
        // Same (test_id, platform, arch, renderer, tolerance) tuple twice.
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r1 | i1 | 2026-08-01\n\
             css/foo | linux | x86_64 | vello_cpu | low | r2 | i2 | 2026-08-02\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].line_no, Some(2));
        assert!(dup[0].message.contains("line 1"));
    }

    #[test]
    fn duplicate_quarantine_different_platform_is_not_duplicate() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r1 | i1 | 2026-08-01\n\
             css/foo | macos | x86_64 | vello_cpu | low | r2 | i2 | 2026-08-02\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(
            !report.issues.iter().any(|i| i.category == Category::Duplicate),
            "unexpected: {:?}",
            report.issues
        );
    }
```

- [ ] **Step 2: テストを走らせて赤を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 4 new tests fail (no Duplicate detection yet). The negative test `duplicate_quarantine_different_platform_is_not_duplicate` will pass trivially.

- [ ] **Step 3: `detect_duplicates` を実装**

`lint.rs` の `load_all` 直下に追加、`run` を差し替え:

```rust
use std::collections::HashMap;

fn data_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content.lines().enumerate().map(|(i, l)| (i + 1, l.trim())).filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

fn detect_dup_by_key<F, K>(
    raw: &str,
    file_display: &str,
    key_of: F,
    message_of: impl Fn(&K, usize) -> String,
) -> Vec<LintIssue>
where
    F: Fn(&str) -> Option<K>,
    K: std::hash::Hash + Eq,
{
    let mut seen: HashMap<K, usize> = HashMap::new();
    let mut issues = Vec::new();
    for (line_no, line) in data_lines(raw) {
        let Some(key) = key_of(line) else { continue };
        if let Some(&first) = seen.get(&key) {
            issues.push(LintIssue {
                category: Category::Duplicate,
                file: file_display.to_owned(),
                line_no: Some(line_no),
                message: message_of(&key, first),
            });
        } else {
            seen.insert(key, line_no);
        }
    }
    issues
}

fn detect_duplicates(loaded: &Loaded, dir: &Path) -> Vec<LintIssue> {
    let mut issues = Vec::new();

    if let Some(raw) = loaded.baseline_raw.as_deref() {
        let path = dir.join("raikiri-baseline.txt").display().to_string();
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| Some(l.to_owned()),
            |k, first| format!("test_id {k:?} already appeared on line {first}"),
        ));
    }

    if let Some(raw) = loaded.deprecated_raw.as_deref() {
        let path = dir.join("deprecated.txt").display().to_string();
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| Some(l.to_owned()),
            |k, first| format!("test_id {k:?} already appeared on line {first}"),
        ));
    }

    if let Some(raw) = loaded.quarantine_raw.as_deref() {
        let path = dir.join("quarantine.txt").display().to_string();
        // Key = first 5 columns (test_id, platform, arch, renderer, tolerance).
        // Silently ignore lines that don't split into >= 5 columns; malformed
        // lines are reported by the parser via detect_malformed.
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| {
                let cols: Vec<&str> = l.split('|').map(str::trim).collect();
                if cols.len() < 5 { return None; }
                Some((
                    cols[0].to_owned(),
                    cols[1].to_owned(),
                    cols[2].to_owned(),
                    cols[3].to_owned(),
                    cols[4].to_owned(),
                ))
            },
            |k, first| format!(
                "(test_id={:?}, platform={:?}, arch={:?}, renderer={:?}, tolerance={:?}) already appeared on line {}",
                k.0, k.1, k.2, k.3, k.4, first
            ),
        ));
    }

    issues
}

pub fn run(dir: &Path, _now: Date) -> LintReport {
    let loaded = load_all(dir);
    let mut issues = loaded.issues.clone();
    issues.extend(detect_duplicates(&loaded, dir));
    LintReport { issues }
}
```

- [ ] **Step 4: テストを走らせて緑を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 9 tests passed (5 previous + 4 new).

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-wpt/src/lint.rs
git commit -m "feat(raikiri-wpt/lint): detect Duplicate via line-level re-scan (raikiri-spike-yps)"
```

---

## Task 4: Conflicting detection (cross-file)

**Files:**
- Modify: `crates/raikiri-wpt/src/lint.rs`

**Interfaces:**
- Consumes: `Loaded::baseline`, `Loaded::deprecated`, `Loaded::quarantine` (parsed sets/entries)
- Produces:
  - Private `fn detect_conflicting(loaded: &Loaded, dir: &Path) -> Vec<LintIssue>`
  - `run` now extends with conflict issues

**Design note:** M3 simple report — any test_id appearing in ≥ 2 of `{deprecated, baseline, quarantine}` yields one issue per (file_a, file_b, test_id) pair. Filter overlap analysis for `quarantine ∩ baseline` is deferred (separate P3 issue).

- [ ] **Step 1: Failing tests — 3 pairwise conflicts**

`mod tests` に追加:

```rust
    #[test]
    fn conflict_deprecated_and_baseline_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/x/y-001\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].message.contains("css/x/y-001"));
        assert!(conflicts[0].message.contains("deprecated.txt") || conflicts[0].file.ends_with("deprecated.txt"));
        assert!(conflicts[0].message.contains("raikiri-baseline.txt") || conflicts[0].file.ends_with("raikiri-baseline.txt"));
    }

    #[test]
    fn conflict_deprecated_and_quarantine_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "deprecated.txt", "css/x/y-001\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/x/y-001 | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn conflict_baseline_and_quarantine_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/x/y-001 | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn no_conflict_when_test_id_appears_in_only_one_file() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/other\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Conflicting));
    }
```

- [ ] **Step 2: テストを走らせて赤を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 3 conflict tests fail.

- [ ] **Step 3: `detect_conflicting` を実装**

`detect_duplicates` の直下に追加、`run` を差し替え:

```rust
fn detect_conflicting(loaded: &Loaded, dir: &Path) -> Vec<LintIssue> {
    use std::collections::BTreeSet;

    let baseline: BTreeSet<&str> = loaded
        .baseline
        .as_ref()
        .map(|b| b.entries.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let deprecated: BTreeSet<&str> = loaded
        .deprecated
        .as_ref()
        .map(|d| d.entries.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let quarantine: BTreeSet<&str> = loaded
        .quarantine
        .as_ref()
        .map(|q| q.entries.iter().map(|e| e.test_id.as_str()).collect())
        .unwrap_or_default();

    let baseline_path = dir.join("raikiri-baseline.txt").display().to_string();
    let deprecated_path = dir.join("deprecated.txt").display().to_string();
    let quarantine_path = dir.join("quarantine.txt").display().to_string();

    let mut issues = Vec::new();
    let mk = |file: &str, other: &str, test_id: &str| LintIssue {
        category: Category::Conflicting,
        file: file.to_owned(),
        line_no: None,
        message: format!(
            "test_id {test_id:?} also appears in {other} (spec §12.10 precedence: resolve via PR)"
        ),
    };
    for id in deprecated.intersection(&baseline) {
        issues.push(mk(&deprecated_path, "raikiri-baseline.txt", id));
    }
    for id in deprecated.intersection(&quarantine) {
        issues.push(mk(&deprecated_path, "quarantine.txt", id));
    }
    for id in baseline.intersection(&quarantine) {
        issues.push(mk(&baseline_path, "quarantine.txt", id));
    }
    issues
}

pub fn run(dir: &Path, _now: Date) -> LintReport {
    let loaded = load_all(dir);
    let mut issues = loaded.issues.clone();
    issues.extend(detect_duplicates(&loaded, dir));
    issues.extend(detect_conflicting(&loaded, dir));
    LintReport { issues }
}
```

- [ ] **Step 4: テストを走らせて緑を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 13 tests passed (9 previous + 4 new; negative test was already passing).

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-wpt/src/lint.rs
git commit -m "feat(raikiri-wpt/lint): detect Conflicting across expectations files (raikiri-spike-yps)"
```

---

## Task 5: Expired detection (with injected `now`)

**Files:**
- Modify: `crates/raikiri-wpt/src/lint.rs`

**Interfaces:**
- Consumes: `Loaded::quarantine` (`QuarantineEntry::added_date: String`)
- Produces:
  - Private `fn detect_expired(loaded: &Loaded, dir: &Path, now: Date) -> Vec<LintIssue>`
  - Parse `added_date` as `time::Date` with format `"[year]-[month]-[day]"` (YYYY-MM-DD)
  - Parse failure → `Category::Malformed` issue
  - `(now - added_date).whole_days() > 90` → `Category::Expired` warning
  - `run` uses injected `now`

**Design note:** the current parser stores `added_date` as `String`. Follow-up bd `raikiri-spike-md0` will migrate to `time::Date`; when that lands, this task's parse step can be removed but the threshold check stays.

- [ ] **Step 1: Failing tests — parse failure, expired, not-yet-expired**

`mod tests` に追加:

```rust
    #[test]
    fn expired_quarantine_older_than_90_days_becomes_warning() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-01-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // 196 days later
        let report = run(dir.path(), now);
        let expired: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Expired).collect();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].line_no, Some(1));
        assert!(expired[0].message.contains("2026-01-01"));
        assert!(expired[0].message.contains("90 days"));
        // Expired-only reports do NOT block CI.
        assert!(!report.has_failures(), "unexpected failure with only Expired: {:?}", report.issues);
    }

    #[test]
    fn quarantine_within_90_days_is_not_expired() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-06-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // 45 days later
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Expired));
    }

    #[test]
    fn expired_boundary_exactly_90_days_is_not_expired() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-04-17\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // exactly 90 days
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Expired));
    }

    #[test]
    fn malformed_added_date_becomes_malformed_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let malformed: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Malformed)
            .collect();
        assert_eq!(malformed.len(), 1);
        assert_eq!(malformed[0].line_no, Some(1));
        assert!(malformed[0].message.contains("added_date"));
        assert!(malformed[0].message.contains("not-a-date"));
    }
```

- [ ] **Step 2: テストを走らせて赤を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 4 new tests fail.

- [ ] **Step 3: `detect_expired` を実装**

`lint.rs` の import に追加:

```rust
use time::{macros::format_description, Duration};
```

`detect_conflicting` の直下に追加、`run` を差し替え:

```rust
fn detect_expired(loaded: &Loaded, dir: &Path, now: Date) -> Vec<LintIssue> {
    let Some(q) = loaded.quarantine.as_ref() else { return Vec::new() };
    let path = dir.join("quarantine.txt").display().to_string();
    let fmt = format_description!("[year]-[month]-[day]");
    let mut issues = Vec::new();
    for (idx, entry) in q.entries.iter().enumerate() {
        // Line number: entries appear in file order, but comments/blank
        // lines shift the parser's index. Recompute by re-scanning the
        // raw content for the idx-th data line.
        let line_no = loaded
            .quarantine_raw
            .as_deref()
            .and_then(|raw| data_lines(raw).nth(idx).map(|(n, _)| n));
        match Date::parse(&entry.added_date, &fmt) {
            Err(e) => issues.push(LintIssue {
                category: Category::Malformed,
                file: path.clone(),
                line_no,
                message: format!(
                    "added_date {:?} is not YYYY-MM-DD ({e})",
                    entry.added_date
                ),
            }),
            Ok(added) => {
                if now - added > Duration::days(90) {
                    issues.push(LintIssue {
                        category: Category::Expired,
                        file: path.clone(),
                        line_no,
                        message: format!(
                            "quarantine entry added on {} is older than 90 days (test_id={:?})",
                            entry.added_date, entry.test_id
                        ),
                    });
                }
            }
        }
    }
    issues
}

pub fn run(dir: &Path, now: Date) -> LintReport {
    let loaded = load_all(dir);
    let mut issues = loaded.issues.clone();
    issues.extend(detect_duplicates(&loaded, dir));
    issues.extend(detect_conflicting(&loaded, dir));
    issues.extend(detect_expired(&loaded, dir, now));
    LintReport { issues }
}
```

- [ ] **Step 4: テストを走らせて緑を確認**

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 17 tests passed (13 previous + 4 new).

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-wpt/src/lint.rs
git commit -m "feat(raikiri-wpt/lint): detect Expired via injected now + time::Date parse (raikiri-spike-yps)"
```

---

## Task 6: Human-readable report formatting + insta snapshot

**Files:**
- Modify: `crates/raikiri-wpt/src/lint.rs`

**Interfaces:**
- Produces:
  - `LintReport::format_human(&self) -> String` — output grouped by `Category`, categories emitted in the order Malformed → Duplicate → Conflicting → Expired
  - Insta snapshot test locking the format

**Design note:** the output is what CI logs will show, so stability matters. Include a summary line (`clean`, or counts).

- [ ] **Step 1: Failing test — `format_human` shape**

`mod tests` に追加:

```rust
    #[test]
    fn format_human_groups_by_category_and_records_counts() {
        let report = LintReport {
            issues: vec![
                LintIssue {
                    category: Category::Malformed,
                    file: "raikiri-baseline.txt".to_owned(),
                    line_no: Some(2),
                    message: "bad".to_owned(),
                },
                LintIssue {
                    category: Category::Duplicate,
                    file: "raikiri-baseline.txt".to_owned(),
                    line_no: Some(3),
                    message: "dup".to_owned(),
                },
                LintIssue {
                    category: Category::Expired,
                    file: "quarantine.txt".to_owned(),
                    line_no: Some(1),
                    message: "old".to_owned(),
                },
            ],
        };
        let out = report.format_human();
        insta::assert_snapshot!("format_human_mixed_categories", out);
    }

    #[test]
    fn format_human_empty_report_snapshot() {
        let out = LintReport::default().format_human();
        insta::assert_snapshot!("format_human_empty", out);
    }
```

- [ ] **Step 2: `format_human` を実装**

`impl LintReport { }` に追加:

```rust
    /// Render as human-readable text grouped by [`Category`].
    ///
    /// Categories always emit in the order Malformed → Duplicate →
    /// Conflicting → Expired for stable CI output. Each issue is one line
    /// of the form `<file>[:<line>]: <message>`. A summary line closes.
    pub fn format_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for cat in [Category::Malformed, Category::Duplicate, Category::Conflicting, Category::Expired] {
            let group: Vec<&LintIssue> = self.issues.iter().filter(|i| i.category == cat).collect();
            if group.is_empty() {
                continue;
            }
            writeln!(out, "== {:?} ({}) ==", cat, group.len()).unwrap();
            for i in group {
                match i.line_no {
                    Some(n) => writeln!(out, "  {}:{}: {}", i.file, n, i.message).unwrap(),
                    None => writeln!(out, "  {}: {}", i.file, i.message).unwrap(),
                }
            }
        }
        if self.is_empty() {
            out.push_str("expectations: clean\n");
        } else {
            let n_fail = self.issues.iter().filter(|i| i.category.is_failure()).count();
            let n_warn = self.issues.iter().filter(|i| !i.category.is_failure()).count();
            writeln!(out, "expectations: {} failure(s), {} warning(s)", n_fail, n_warn).unwrap();
        }
        out
    }
```

- [ ] **Step 3: snapshot を初回受理**

```bash
cargo test -p raikiri-wpt --lib lint::tests::format_human 2>&1 | tail -20
```

Expected: 2 tests fail with `snapshot missing` (insta creates `.pending-snap` files). Review and accept:

```bash
cargo insta review --workspace 2>&1
# Or accept all pending snapshots for this test module:
cargo insta accept --workspace
```

Then re-run:

```bash
cargo test -p raikiri-wpt --lib lint::
```

Expected: 19 tests passed (17 previous + 2 new).

- [ ] **Step 4: Commit**

```bash
git add crates/raikiri-wpt/src/lint.rs crates/raikiri-wpt/src/snapshots/
git commit -m "feat(raikiri-wpt/lint): format_human groups issues by category + insta snapshots (raikiri-spike-yps)"
```

---

## Task 7: `validate-expectations` bin wrapper + integration test

**Files:**
- Modify: `crates/raikiri-wpt/src/bin/validate-expectations.rs`
- Create: `crates/raikiri-wpt/tests/validate_expectations_bin.rs`

**Interfaces:**
- Consumes: `raikiri_wpt::lint::{run, LintReport}`, workspace `expectations/` directory resolution (mirror `ExpectationSet::load_from_workspace_root` logic — `CARGO_MANIFEST_DIR/../..`)
- Produces:
  - `validate-expectations` bin: no CLI args in M3. Uses workspace `expectations/` dir, `time::OffsetDateTime::now_utc().date()` for `now`, prints report to stdout, exits 0 (clean or Expired-only) or 2 (any failure)
  - Bin integration test that stages a fixture directory and runs the compiled bin against it via env var `RAIKIRI_WPT_EXPECTATIONS_DIR` (test-only override)

**Design note:** the bin resolves the expectations directory in this order to keep tests self-contained: `env RAIKIRI_WPT_EXPECTATIONS_DIR` if set → otherwise the workspace-root `expectations/` directory (via `CARGO_MANIFEST_DIR`).

- [ ] **Step 1: Bin を rewrite**

`crates/raikiri-wpt/src/bin/validate-expectations.rs` を置換:

```rust
//! Validate `expectations/*.txt` files (spec §12.10).
//!
//! Detects Malformed / Duplicate / Conflicting (CI-blocking) and Expired
//! (warning) issues. Runs in CI on every PR via the m1.12 GitHub Actions
//! workflow.
//!
//! Exit codes:
//! - `0`: clean, or only [`raikiri_wpt::lint::Category::Expired`] warnings
//! - `2`: any Malformed / Duplicate / Conflicting failure

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::lint;
use time::OffsetDateTime;

fn expectations_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("RAIKIRI_WPT_EXPECTATIONS_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("expectations")
}

fn main() -> ExitCode {
    let dir = expectations_dir();
    let now = OffsetDateTime::now_utc().date();
    let report = lint::run(&dir, now);
    print!("{}", report.format_human());
    if report.has_failures() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
```

- [ ] **Step 2: `cargo build --bin validate-expectations` で確認**

```bash
cargo build -p raikiri-wpt --bin validate-expectations
```

Expected: builds clean, no warnings.

- [ ] **Step 3: Integration test を書く**

`crates/raikiri-wpt/tests/validate_expectations_bin.rs` を新規作成:

```rust
//! Bin-level integration test for `validate-expectations`.
//!
//! Uses `assert_cmd` to invoke the compiled bin against a `tempfile`-
//! staged expectations directory via the `RAIKIRI_WPT_EXPECTATIONS_DIR`
//! env var override.

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

fn stage_header_only_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "tracked-wpt.txt",
        "known-issues.txt",
        "raikiri-baseline.txt",
        "quarantine.txt",
        "deprecated.txt",
    ] {
        fs::write(dir.path().join(name), "# header\n").unwrap();
    }
    dir
}

#[test]
fn bin_exits_zero_on_clean_expectations() {
    let dir = stage_header_only_dir();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("clean"), "unexpected stdout: {out}");
}

#[test]
fn bin_exits_two_on_malformed_quarantine() {
    let dir = stage_header_only_dir();
    // Overwrite quarantine.txt with a malformed line (3 cols instead of 8).
    fs::write(dir.path().join("quarantine.txt"), "css/foo | linux | x86_64\n").unwrap();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(2);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Malformed"), "unexpected stdout: {out}");
}

#[test]
fn bin_exits_zero_when_only_expired_warnings() {
    let dir = stage_header_only_dir();
    // A quarantine entry with a very old added_date (well beyond 90 days,
    // relative to any plausible bin-invocation "now").
    fs::write(
        dir.path().join("quarantine.txt"),
        "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2000-01-01\n",
    )
    .unwrap();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Expired"), "unexpected stdout: {out}");
    assert!(out.contains("warning(s)"), "unexpected stdout: {out}");
}
```

- [ ] **Step 4: Bin test を実行**

```bash
cargo test -p raikiri-wpt --test validate_expectations_bin
```

Expected: 3 tests passed.

- [ ] **Step 5: 全 test + clippy + fmt で緑を確認**

```bash
cargo fmt --check
cargo clippy -p raikiri-wpt --all-targets -- -D warnings
cargo test -p raikiri-wpt
```

Expected: all green.

- [ ] **Step 6: workspace `expectations/` に対して bin を走らせて sanity check**

```bash
cargo run -p raikiri-wpt --bin validate-expectations
```

Expected: exit 0, output `expectations: clean` (workspace files are header-only).

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri-wpt/src/bin/validate-expectations.rs crates/raikiri-wpt/tests/validate_expectations_bin.rs
git commit -m "feat(raikiri-wpt): validate-expectations bin wires lint::run with exit codes (raikiri-spike-yps)"
```

---

## Self-Review Checklist (do this before handing off)

**1. Spec coverage against §12.10:**
- [ ] Malformed — Task 2 (parser errors surfaced) + Task 5 (bad added_date)
- [ ] Duplicate — Task 3 (baseline/deprecated single-column, quarantine 5-tuple)
- [ ] Conflicting — Task 4 (deprecated∩baseline, deprecated∩quarantine, baseline∩quarantine)
- [ ] Expired — Task 5 (90-day threshold, injected `now`, exit-0 warning)
- [ ] Fail-fast in CI — Task 7 (exit code 2 for M/D/C)
- [ ] Bin runnable via `cargo run --bin validate-expectations` — Task 7

**2. Placeholder scan:** none — every task has full code, exact paths, exact commands.

**3. Type consistency:**
- `LintReport::has_failures`, `is_empty`, `format_human` — used consistently in Task 7 bin
- `LintIssue { category, file, line_no, message }` — same shape across all tasks
- `lint::run(dir: &Path, now: time::Date)` — signature stable from Task 1 through Task 7
- `Loaded` struct — extended only via new fields, no field renames

**4. Global constraint compliance:**
- MSRV 1.89 — `time = "0.3"` supports MSRV 1.67 (well within limits)
- `unsafe_code = "deny"` — no `unsafe` in any task
- `missing_docs = "warn"` — every public item has `///` doc
- No changes to `expectations.rs` public API — verified

---

## Execution Handoff

Plan complete. Next step per `blueprint:impl`: execute via **Subagent-Driven Development** (default). If Parallel Session preferred, open a new session in `/home/mitz/worktrees/github.com/mitsuru/raikiri-spike/worktree-raikiri-spike-yps` and use `superpowers:executing-plans`.
