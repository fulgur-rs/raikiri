# raikiri-spike-rpd Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Quarantine::parse` を per-row error accumulation API に breaking 変更し、single malformed row が同 file 内 valid entries の lint check (expired/conflicting) を suppress する回帰を除去する。

**Architecture:** `Quarantine::parse` の signature を `Result<Self, ExpectError>` → `(Quarantine, Vec<ExpectError>)` (outer Result なし) に変更。全 6 error site (column-count / platform / arch / renderer / tolerance / added_date) を per-row `continue`+`Vec::push` パターンに書き換える。`Quarantine::load` は outer Result を I/O 専用に絞る (`Result<(Quarantine, Vec<ExpectError>), ExpectError>`)。`lint::load_all` は inner Vec を LintIssue に展開する caller に更新、`ExpectationSet::load_from` は first-error fallback で古い outer signature を保持する。

**Tech Stack:** Rust (workspace crate `raikiri-wpt`), `time` crate for `Date`, `tempfile` for lint テスト fixture。

## Global Constraints

- Scope: `crates/raikiri-wpt/` のみ。ワークスペースの他 crate は触らない。
- Date crate: workspace 唯一の `time` を使う。`chrono` / `jiff` は追加しない。
- `Quarantine::parse` の新 signature は `(Quarantine, Vec<ExpectError>)` (outer Result なし)。
- `Quarantine::load` の新 signature は `Result<(Quarantine, Vec<ExpectError>), ExpectError>` (outer は I/O 専用)。
- `ExpectationSet::load_from` の outer signature (`Result<Self, ExpectError>`) は保持する — 破壊しない。
- 同 row 内で複数 column が失敗しても最初の失敗のみ error に記録 (short-circuit per-row)、row 越しには継続。
- 検証: `cargo test -p raikiri-wpt`, `cargo fmt --check -p raikiri-wpt`, `cargo clippy -p raikiri-wpt --all-targets -- -D warnings` すべて clean。
- Related follow-up: raikiri-spike-9k6 (calendar-invalid negative tests + `format_description!` inline) — 本 plan では触らない。

---

### Task 1: `Quarantine::parse` / `Quarantine::load` を per-row error accumulation に移行 (signature + body + all callers + existing tests)

**Files:**
- Modify: `crates/raikiri-wpt/src/expectations.rs` (`Quarantine::parse`, `Quarantine::load`, `ExpectationSet::load_from`, 既存 test 5 個, `QuarantineEntry.added_date` の doc)
- Modify: `crates/raikiri-wpt/src/lint.rs` (`load_all` の Quarantine 部分)
- Test: `crates/raikiri-wpt/src/expectations.rs::tests::quarantine_accumulates_multiple_row_errors` (new)

**Interfaces:**
- Consumes: `ExpectError::{Io, MalformedLine, UnknownEnum}` (既存), `time::Date::parse`, `format_description!("[year]-[month]-[day]")`。
- Produces:
  - `pub fn Quarantine::parse(content: &str, file_name: &str) -> (Quarantine, Vec<ExpectError>)`
  - `pub fn Quarantine::load(path: &Path) -> Result<(Quarantine, Vec<ExpectError>), ExpectError>`
  - `Task 2` はこの 2 signature に依存して新 regression test を書く。

- [ ] **Step 1: TDD anchor テストを追加 (compile fail 想定)**

`crates/raikiri-wpt/src/expectations.rs` の `mod tests` の末尾 (最後の quarantine テスト `quarantine_rejects_malformed_added_date` の後、`baseline_deduplicates_via_hashset` の前後どちらかの近傍) に以下の test を追加:

```rust
#[test]
fn quarantine_accumulates_multiple_row_errors() {
    // 3 rows: valid, malformed (wrong column count), valid.
    // Expect the malformed row to accumulate as an error while both
    // valid rows survive as entries.
    let content = "\
css/a | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/bad | linux | x86_64
css/b | macos | aarch64 | skia | high | r | i | 2026-08-02
";
    let (q, errors) = Quarantine::parse(content, "q.txt");
    assert_eq!(q.entries.len(), 2, "expected 2 valid entries, got {:?}", q.entries);
    assert_eq!(q.entries[0].test_id, "css/a");
    assert_eq!(q.entries[1].test_id, "css/b");
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ExpectError::MalformedLine { line_no, reason, .. } => {
            assert_eq!(*line_no, 2);
            assert!(reason.contains("expected 8"), "got reason: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}
```

- [ ] **Step 2: 追加 test を build して compile 失敗を確認**

Run: `cargo build --tests -p raikiri-wpt 2>&1 | tail -30`
Expected: 追加 test を含む `expectations::tests` で `Quarantine::parse` が `Result` を返すので tuple destructure (`(q, errors) = ...`) が type-mismatch でコンパイル失敗。この失敗が TDD anchor。

- [ ] **Step 3: `Quarantine::parse` の signature と body を per-row accumulation に書き換え**

`crates/raikiri-wpt/src/expectations.rs` の `Quarantine::parse` (現在 L378-435 相当) を以下に置換:

```rust
impl Quarantine {
    /// Parse `quarantine.txt` content (8 pipe-delimited columns per line).
    ///
    /// Row-level validation errors (bad column count, unknown enum values,
    /// malformed `added_date`) are accumulated in the returned
    /// `Vec<ExpectError>` and the offending rows are skipped; other valid
    /// rows are still surfaced in the returned [`Quarantine`]. This lets
    /// the lint pass surface every issue in a file at once instead of
    /// aborting on the first malformed row.
    pub fn parse(content: &str, file_name: &str) -> (Self, Vec<ExpectError>) {
        let mut entries = Vec::new();
        let mut errors: Vec<ExpectError> = Vec::new();
        let added_date_fmt = format_description!("[year]-[month]-[day]");
        for (line_no, line) in iter_data_lines(content) {
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            if cols.len() != 8 {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("expected 8 pipe-delimited columns, got {}", cols.len()),
                });
                continue;
            }
            let Some(platform) = PlatformFilter::parse(cols[1]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "platform",
                    value: cols[1].to_owned(),
                });
                continue;
            };
            let Some(arch) = ArchFilter::parse(cols[2]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "arch",
                    value: cols[2].to_owned(),
                });
                continue;
            };
            let Some(renderer) = RendererFilter::parse(cols[3]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "renderer",
                    value: cols[3].to_owned(),
                });
                continue;
            };
            let Some(tolerance) = ToleranceFilter::parse(cols[4]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "tolerance",
                    value: cols[4].to_owned(),
                });
                continue;
            };
            let added_date = match Date::parse(cols[7], &added_date_fmt) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(ExpectError::MalformedLine {
                        file: file_name.to_owned(),
                        line_no,
                        reason: format!("added_date {:?} is not YYYY-MM-DD ({e})", cols[7]),
                    });
                    continue;
                }
            };
            entries.push(QuarantineEntry {
                test_id: cols[0].to_owned(),
                platform,
                arch,
                renderer,
                tolerance,
                reason: cols[5].to_owned(),
                issue_link: cols[6].to_owned(),
                added_date,
            });
        }
        (Self { entries }, errors)
    }

    /// Read and parse `quarantine.txt` from `path`. The outer [`Result`]
    /// surfaces filesystem I/O errors only; per-row parse errors are
    /// returned in the inner tuple's `Vec<ExpectError>`.
    pub fn load(path: &Path) -> Result<(Self, Vec<ExpectError>), ExpectError> {
        let content = read_file(path)?;
        Ok(Self::parse(&content, &path.display().to_string()))
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
```

- [ ] **Step 4: `QuarantineEntry.added_date` の doc comment を新 error path 名に合わせて微調整**

`crates/raikiri-wpt/src/expectations.rs` の `QuarantineEntry.added_date` の doc (L265-269 相当) を以下に置換:

```rust
    /// Date the entry was added, in YYYY-MM-DD (ISO 8601) form. Parsed and
    /// validated at [`Quarantine::parse`] time via
    /// [`time::Date::parse`]; a malformed date is recorded in the
    /// `Vec<ExpectError>` returned alongside the parsed [`Quarantine`] as
    /// [`ExpectError::MalformedLine`], and the offending row is skipped.
    pub added_date: Date,
```

- [ ] **Step 5: `ExpectationSet::load_from` を first-error fallback に更新**

`crates/raikiri-wpt/src/expectations.rs` の `ExpectationSet::load_from` (現在 L57-65 相当) の `quarantine:` 行を置換:

現状:
```rust
    pub fn load_from(dir: &Path) -> Result<Self, ExpectError> {
        Ok(Self {
            tracked: TrackedWpt::load(&dir.join("tracked-wpt.txt"))?,
            known_issues: KnownIssues::load(&dir.join("known-issues.txt"))?,
            baseline: Baseline::load(&dir.join("raikiri-baseline.txt"))?,
            quarantine: Quarantine::load(&dir.join("quarantine.txt"))?,
            deprecated: Deprecated::load(&dir.join("deprecated.txt"))?,
        })
    }
```

新 (load 順序: tracked → known_issues → baseline → quarantine → deprecated を保持する形):
```rust
    pub fn load_from(dir: &Path) -> Result<Self, ExpectError> {
        let tracked = TrackedWpt::load(&dir.join("tracked-wpt.txt"))?;
        let known_issues = KnownIssues::load(&dir.join("known-issues.txt"))?;
        let baseline = Baseline::load(&dir.join("raikiri-baseline.txt"))?;
        let (quarantine, quarantine_errors) =
            Quarantine::load(&dir.join("quarantine.txt"))?;
        if let Some(e) = quarantine_errors.into_iter().next() {
            return Err(e);
        }
        let deprecated = Deprecated::load(&dir.join("deprecated.txt"))?;
        Ok(Self {
            tracked,
            known_issues,
            baseline,
            quarantine,
            deprecated,
        })
    }
```

- [ ] **Step 6: `lint.rs::load_all` の Quarantine 分岐を Vec 展開型に更新**

`crates/raikiri-wpt/src/lint.rs` の L211-227 相当 (quarantine_path 部分) を置換:

現状:
```rust
    let quarantine_path = dir.join("quarantine.txt");
    match std::fs::read_to_string(&quarantine_path) {
        Ok(raw) => {
            match Quarantine::parse(&raw, &quarantine_path.display().to_string()) {
                Ok(v) => out.quarantine = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(
                    e,
                    quarantine_path.display().to_string(),
                )),
            }
            out.quarantine_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(
            ExpectError::Io(e),
            quarantine_path.display().to_string(),
        )),
    }
```

新:
```rust
    let quarantine_path = dir.join("quarantine.txt");
    match std::fs::read_to_string(&quarantine_path) {
        Ok(raw) => {
            let (v, errors) =
                Quarantine::parse(&raw, &quarantine_path.display().to_string());
            for e in errors {
                out.issues.push(expect_error_to_issue(
                    e,
                    quarantine_path.display().to_string(),
                ));
            }
            out.quarantine = Some(v);
            out.quarantine_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(
            ExpectError::Io(e),
            quarantine_path.display().to_string(),
        )),
    }
```

- [ ] **Step 7: `expectations.rs` の既存 5 test を新 signature に migrate**

同 file の `mod tests` にある以下 5 test を書き換える。行位置は 現状 (L537 empty, L595 8-col, L611 wildcards, L622 wrong-col, L637 unknown-platform, L650 bad-date) 相当。

**7a. `empty_content_yields_empty_sets` の Quarantine 行:**

現状: `let q = Quarantine::parse("# 8-col format follows\n", "quarantine.txt").unwrap();`

新: `let (q, q_errs) = Quarantine::parse("# 8-col format follows\n", "quarantine.txt"); assert!(q_errs.is_empty()); assert!(q.is_empty());`

(既存の `assert!(q.is_empty());` は残し、その直前に `assert!(q_errs.is_empty());` を追加する形でも可。)

**7b. `quarantine_parses_8_cols_and_enum_values`:**

現状:
```rust
    let q = Quarantine::parse(content, "q.txt").unwrap();
    assert_eq!(q.entries.len(), 1);
    let e = &q.entries[0];
    // ... field assertions ...
```

新:
```rust
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    assert_eq!(q.entries.len(), 1);
    let e = &q.entries[0];
    // ... field assertions unchanged ...
```

**7c. `quarantine_accepts_wildcards`:**

現状: `let q = Quarantine::parse(content, "q.txt").unwrap();`

新: `let (q, errs) = Quarantine::parse(content, "q.txt"); assert!(errs.is_empty(), "expected no errors, got {errs:?}");`

**7d. `quarantine_rejects_wrong_col_count`:**

現状:
```rust
    let content = "css/foo | linux | x86_64\n"; // 3 cols
    let err = Quarantine::parse(content, "q.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine { line_no, reason, .. } => {
            assert_eq!(line_no, 1);
            assert!(reason.contains("expected 8"));
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
```

新:
```rust
    let content = "css/foo | linux | x86_64\n"; // 3 cols
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine { line_no, reason, .. } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("expected 8"));
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
```

**7e. `quarantine_rejects_unknown_platform`:**

現状:
```rust
    let err = Quarantine::parse(content, "q.txt").unwrap_err();
    match err {
        ExpectError::UnknownEnum { field, value, .. } => {
            assert_eq!(field, "platform");
            assert_eq!(value, "plan9");
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
```

新:
```rust
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::UnknownEnum { field, value, .. } => {
            assert_eq!(*field, "platform");
            assert_eq!(value, "plan9");
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
```

**7f. `quarantine_rejects_malformed_added_date`:**

現状:
```rust
    let err = Quarantine::parse(content, "q.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine { line_no, reason, .. } => {
            assert_eq!(line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("not-a-date"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
```

新:
```rust
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine { line_no, reason, .. } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("not-a-date"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
```

- [ ] **Step 8: `cargo test -p raikiri-wpt` を実行し、既存 test + Task 1 anchor が全 pass することを確認**

Run: `cargo test -p raikiri-wpt 2>&1 | tail -30`
Expected: 全 unit test + integration test + doc-test pass、new `quarantine_accumulates_multiple_row_errors` を含む。テスト数は元の 35 unit + 3 integration + 1 doc = 39 に 1 追加された 40 (最低)。

- [ ] **Step 9: Commit**

```bash
git add crates/raikiri-wpt/src/expectations.rs crates/raikiri-wpt/src/lint.rs
git commit -m "refactor(raikiri-wpt): Quarantine::parse per-row error accumulation (rpd)"
```

---

### Task 2: 追加 regression tests (enum-error continuation + mixed Malformed+Expired)

**Files:**
- Test: `crates/raikiri-wpt/src/expectations.rs::tests::quarantine_continues_after_enum_error` (new)
- Test: `crates/raikiri-wpt/src/lint.rs::tests::mixed_malformed_and_valid_expired_surfaces_both` (new)

**Interfaces:**
- Consumes: Task 1 の `Quarantine::parse(&str, &str) -> (Quarantine, Vec<ExpectError>)` と `lint::run(&Path, Date) -> LintReport`。

- [ ] **Step 1: `expectations.rs` に enum-error continuation test を追加**

`crates/raikiri-wpt/src/expectations.rs` の `mod tests` 内 (Task 1 で追加した `quarantine_accumulates_multiple_row_errors` の直後) に追加:

```rust
#[test]
fn quarantine_continues_after_enum_error() {
    // Bad platform followed by a valid row. Ensure the valid row is
    // still surfaced in entries and the enum error is captured.
    let content = "\
css/bad | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/ok | macos | aarch64 | skia | high | r | i | 2026-08-02
";
    let (q, errors) = Quarantine::parse(content, "q.txt");
    assert_eq!(q.entries.len(), 1);
    assert_eq!(q.entries[0].test_id, "css/ok");
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ExpectError::UnknownEnum { field, value, line_no, .. } => {
            assert_eq!(*field, "platform");
            assert_eq!(value, "plan9");
            assert_eq!(*line_no, 1);
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}
```

- [ ] **Step 2: `lint.rs` に mixed Malformed + Expired test を追加**

`crates/raikiri-wpt/src/lint.rs` の `mod tests` 内 (最後の `malformed_added_date_becomes_malformed_lint_issue` の後、`format_human_groups_by_category_and_records_counts` の前) に追加:

```rust
#[test]
fn mixed_malformed_and_valid_expired_surfaces_both() {
    // A malformed row (wrong column count) plus a valid but > 90 days
    // old row must produce both a Malformed and an Expired lint issue —
    // the malformed row must not swallow the expired check for the
    // surviving valid entry.
    let dir = header_only_dir();
    write(
        dir.path(),
        "quarantine.txt",
        "\
css/bad | linux | x86_64
css/old | linux | x86_64 | vello_cpu | low | r | i | 2026-01-01
",
    );
    let now = time::macros::date!(2026 - 07 - 16);
    let report = run(dir.path(), now);

    let malformed: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.category == Category::Malformed)
        .collect();
    assert_eq!(malformed.len(), 1, "got: {malformed:?}");
    assert_eq!(malformed[0].line_no, Some(1));
    assert!(malformed[0].message.contains("expected 8"), "got: {}", malformed[0].message);

    let expired: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.category == Category::Expired)
        .collect();
    assert_eq!(expired.len(), 1, "got: {expired:?}");
    assert!(
        expired[0].message.contains("css/old"),
        "got: {}",
        expired[0].message
    );
    assert_eq!(expired[0].line_no, Some(2));
}
```

- [ ] **Step 3: `cargo test -p raikiri-wpt` を実行**

Run: `cargo test -p raikiri-wpt 2>&1 | tail -30`
Expected: 全 test pass、新 test 2 個 (`quarantine_continues_after_enum_error`, `mixed_malformed_and_valid_expired_surfaces_both`) が含まれる。

- [ ] **Step 4: Commit**

```bash
git add crates/raikiri-wpt/src/expectations.rs crates/raikiri-wpt/src/lint.rs
git commit -m "test(raikiri-wpt): row-accumulation regression tests (rpd)"
```

---

### Task 3: fmt + clippy verification

**Files:** なし (verification のみ、fmt が変更を発生させた場合のみ commit)

**Interfaces:** Task 1 + Task 2 の全変更を対象。

- [ ] **Step 1: `cargo fmt --check -p raikiri-wpt`**

Run: `cargo fmt --check -p raikiri-wpt 2>&1`
Expected: no output (clean)。差分が出た場合は `cargo fmt -p raikiri-wpt` を実行し、fmt 差分を stage して commit する:
```bash
cargo fmt -p raikiri-wpt
git add crates/raikiri-wpt/src/
git commit -m "style(raikiri-wpt): cargo fmt (rpd)"
```

- [ ] **Step 2: `cargo clippy -p raikiri-wpt --all-targets -- -D warnings`**

Run: `cargo clippy -p raikiri-wpt --all-targets -- -D warnings 2>&1 | tail -30`
Expected: warning 0 個、exit code 0。

- [ ] **Step 3: `cargo test -p raikiri-wpt` を最終 pass 確認 (念のため)**

Run: `cargo test -p raikiri-wpt 2>&1 | tail -30`
Expected: 全 test pass。

- [ ] **Step 4: Acceptance criteria の対応表を照合する (手動 review)**

以下を手で確認:
1. ✓ `Quarantine::parse` signature が `(Quarantine, Vec<ExpectError>)`
2. ✓ `Quarantine::load` signature が `Result<(Quarantine, Vec<ExpectError>), ExpectError>`
3. ✓ 6 error site すべてが per-row `continue`+`push` パターン
4. ✓ `lint.rs::load_all` が Vec を LintIssue に展開する caller
5. ✓ `ExpectationSet::load_from` の外 signature が `Result<Self, ExpectError>` のまま (first-error fallback)
6. ✓ 新 regression test `quarantine_accumulates_multiple_row_errors` が存在
7. ✓ 新 regression test `quarantine_continues_after_enum_error` が存在
8. ✓ 新 regression test `mixed_malformed_and_valid_expired_surfaces_both` が存在
9. ✓ `cargo test -p raikiri-wpt` 全 pass
10. ✓ `cargo fmt --check` + `cargo clippy -D warnings` clean

不足があれば該当 Task に戻る。
