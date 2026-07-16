# raikiri-spike-md0: QuarantineEntry.added_date String → time::Date Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `raikiri-wpt` の `QuarantineEntry.added_date` を `String` から `time::Date` に昇格し、`Quarantine::parse` で YYYY-MM-DD 形式検証を強化。`lint::detect_expired` の重複した `Date::parse` ブランチを削除する。

**Architecture:** parse-time validation でセマンティクス invariant を型で表現する pure refactor。フィールド型を `String` → `time::Date` に変更し、`Quarantine::parse` に `time::Date::parse(&format_description!("[year]-[month]-[day]"))` を追加。malformed date は `ExpectError::MalformedLine` として parse 側で surfacing、`lint::detect_expired` 側は正常系のみ (90 日閾値の比較) に責務を集約。

**Tech Stack:** Rust 2024 edition · `time` 0.3 (workspace dep、features=`["std","parsing","macros"]`) · `insta` (snapshot tests) · `assert_cmd` + `tempfile` (bin integration tests)

## Global Constraints

- workspace 統一の date crate は **`time` 0.3**。chrono/jiff は使用禁止 (memory: `m3-yps-time-crate-selection`)。
- `time` は `default-features = false, features = ["std","parsing","macros"]` の設定。追加 feature が必要になったら workspace の `Cargo.toml` を更新する (不要のはず)。
- date format literal は **compile-time** の `format_description!("[year]-[month]-[day]")` マクロで表現する (`parse` は Result を返すが format 文字列自体は compile-time 検証)。
- `expectations/quarantine.txt` の既存 entries は変更禁止 — YYYY-MM-DD の既存 format と bit-互換な移行であることが AC #6。
- lint message の shape は既存契約を維持: `expired_quarantine_older_than_90_days_becomes_warning` テストは `expired[0].message.contains("2026-01-01")` を assert しているので、`entry.added_date` の Display 出力が同じ表現 (`2026-01-01`) であることを確認。time::Date の Display は ISO 8601 (`[year]-[month]-[day]`) を返すため契約は保たれる。
- 変更対象 crate は **raikiri-wpt のみ**。他 crate へ波及なし。
- 実装完了時に `cargo test -p raikiri-wpt` 全 pass (baseline: unit 34 + bin 3 + doc 1 = **38 tests**。新規テスト 1 個追加で unit 35 になる想定)、`cargo fmt --check -p raikiri-wpt` clean、`cargo clippy -p raikiri-wpt --all-targets -- -D warnings` clean。

---

## File Structure

**Modify:**
- `crates/raikiri-wpt/src/expectations.rs` — `QuarantineEntry.added_date` 型変更、`Quarantine::parse` に date parse 追加、field doc 更新、既存 test 1 個更新、新規 test 1 個追加
- `crates/raikiri-wpt/src/lint.rs` — `detect_expired` から `Date::parse` ブランチ削除、doc 更新

**Do not touch:**
- `crates/raikiri-wpt/tests/validate_expectations_bin.rs` — bin 側は入力 `quarantine.txt` を書くだけで型を扱わない。既存 3 テストは無変更で pass するはず (`bin_exits_two_on_malformed_quarantine` はカラム数不足のケースなので影響なし)。
- `crates/raikiri-wpt/src/bin/validate-expectations.rs` — `lint::run` を呼ぶだけで added_date に触らない。無変更で pass。
- `crates/raikiri-wpt/src/snapshots/*.snap` — snapshot 内容は added_date に触れていない。無変更。
- `expectations/quarantine.txt` — fmt 互換 (AC #6)、無変更。
- `crates/raikiri-wpt/Cargo.toml` — `time = { workspace = true }` は既に設定済み。無変更。

---

## Task 1: Migrate `QuarantineEntry.added_date` to `time::Date` with parse-time validation

**Files:**
- Modify: `crates/raikiri-wpt/src/expectations.rs:263-267` (field), `crates/raikiri-wpt/src/expectations.rs:373-437` (Quarantine::parse), `crates/raikiri-wpt/src/expectations.rs:585-598` (existing unit test), `crates/raikiri-wpt/src/expectations.rs` (add new unit test in `mod tests`)
- Modify: `crates/raikiri-wpt/src/lint.rs:16` (import — remove `Duration`, add `format_description` if not already there; actually `format_description` は既存 import 済み), `crates/raikiri-wpt/src/lint.rs:396-443` (detect_expired) 
- Test: unit tests inline in the same files

**Interfaces:**
- Produces: `QuarantineEntry.added_date: time::Date` (was `String`)
- Produces: `Quarantine::parse(content, file_name)` now returns `Err(ExpectError::MalformedLine { reason: "added_date {:?} is not YYYY-MM-DD (...)", ... })` when col[7] is not YYYY-MM-DD format
- Consumes (unchanged): `time::Date::parse` from workspace `time` 0.3 crate
- Consumes (unchanged): `format_description!("[year]-[month]-[day]")` compile-time macro

- [ ] **Step 1: Add failing TDD test for parse-time added_date validation**

Add this test at the bottom of `crates/raikiri-wpt/src/expectations.rs::tests` (immediately after the existing `quarantine_rejects_unknown_platform` test, around line 638):

```rust
#[test]
fn quarantine_rejects_malformed_added_date() {
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n";
    let err = Quarantine::parse(content, "q.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("not-a-date"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails (RED)**

Run: `cargo test -p raikiri-wpt --lib expectations::tests::quarantine_rejects_malformed_added_date`
Expected: FAIL — currently `Quarantine::parse` returns `Ok(...)` for any string in col[7]. Error message will be `panicked at 'called Result::unwrap_err() on an Ok value: Quarantine { entries: [QuarantineEntry { ... added_date: "not-a-date" }] }'`.

- [ ] **Step 3: Update `QuarantineEntry.added_date` field type**

Replace `crates/raikiri-wpt/src/expectations.rs:263-267`:

```rust
    /// Date the entry was added, in YYYY-MM-DD (ISO 8601) form. Parsed and
    /// validated at [`Quarantine::parse`] time via
    /// [`time::Date::parse`]; a malformed date surfaces as
    /// [`ExpectError::MalformedLine`].
    pub added_date: Date,
```

Add the import at the top of the file (after line 16):

```rust
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use time::{Date, macros::format_description};
```

- [ ] **Step 4: Add date parse in `Quarantine::parse`**

Replace the `entries.push(QuarantineEntry { ... })` block at `crates/raikiri-wpt/src/expectations.rs:413-422`:

```rust
            let added_date_fmt = format_description!("[year]-[month]-[day]");
            let added_date = Date::parse(cols[7], &added_date_fmt).map_err(|e| {
                ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("added_date {:?} is not YYYY-MM-DD ({e})", cols[7]),
                }
            })?;
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
```

- [ ] **Step 5: Update existing `quarantine_parses_8_cols_and_enum_values` test**

Replace `crates/raikiri-wpt/src/expectations.rs:597`:

Old line:
```rust
        assert_eq!(e.added_date, "2026-08-01");
```

New line:
```rust
        assert_eq!(e.added_date, time::macros::date!(2026 - 08 - 01));
```

- [ ] **Step 6: Simplify `lint::detect_expired` to drop `Date::parse` branch**

Replace `crates/raikiri-wpt/src/lint.rs:396-443` (the entire `detect_expired` function, including doc comment) with:

```rust
/// Scan the `quarantine.txt` entries for entries older than 90 days
/// relative to `now` (→ [`Category::Expired`], warning-only).
///
/// Parse failures for `added_date` are already surfaced upstream as
/// [`Category::Malformed`] via [`crate::expectations::Quarantine::parse`],
/// so this function only handles the 90-day threshold check.
fn detect_expired(loaded: &Loaded, dir: &Path, now: Date) -> Vec<LintIssue> {
    let Some(q) = loaded.quarantine.as_ref() else {
        return Vec::new();
    };
    let path = dir.join("quarantine.txt").display().to_string();
    let mut issues = Vec::new();
    for (idx, entry) in q.entries.iter().enumerate() {
        // Line number: entries appear in file order, but comments/blank
        // lines shift the parser's index. Recompute by re-scanning the
        // raw content for the idx-th data line.
        let line_no = loaded
            .quarantine_raw
            .as_deref()
            .and_then(|raw| data_lines(raw).nth(idx).map(|(n, _)| n));
        if now - entry.added_date > Duration::days(90) {
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
    issues
}
```

Then update the imports at `crates/raikiri-wpt/src/lint.rs:16`:

Old:
```rust
use time::{Date, Duration, macros::format_description};
```

New:
```rust
use time::{Date, Duration};
```

(`format_description` macro is no longer used in `lint.rs`; the `date!` macro is only used in tests via the fully-qualified `time::macros::date!`.)

- [ ] **Step 7: Build to catch any compile errors before running tests**

Run: `cargo build -p raikiri-wpt --all-targets`
Expected: Clean build. Any leftover reference to `entry.added_date: String` semantics (e.g. `&entry.added_date` where `&str` was expected) will surface here. The main call site — `format!("...{}...", entry.added_date, ...)` — uses `Display`, which `time::Date` implements as ISO 8601 (YYYY-MM-DD), so the message shape is preserved.

- [ ] **Step 8: Run the full raikiri-wpt test suite**

Run: `cargo test -p raikiri-wpt`
Expected — all pass (baseline 34 unit + 3 bin + 1 doc, plus the new 1 unit test = **35 unit + 3 bin + 1 doc = 39 tests**). Specifically:
- `quarantine_rejects_malformed_added_date` (Task 1 new test) → PASS
- `quarantine_parses_8_cols_and_enum_values` → PASS (with `date!` macro)
- `malformed_added_date_becomes_malformed_lint_issue` (lint.rs:821) → PASS. This test writes `"...| not-a-date\n"` to `quarantine.txt` then calls `run(dir, now)`. Now `Quarantine::parse` fails at load time with `ExpectError::MalformedLine { reason: "added_date \"not-a-date\" is not YYYY-MM-DD (...)", ... }`, `load_all` catches it via `expect_error_to_issue` and produces `LintIssue { category: Malformed, message: "malformed line (added_date \"not-a-date\" is not YYYY-MM-DD (...))" }`. Assertions `contains("added_date")` and `contains("not-a-date")` both satisfied.
- `expired_quarantine_older_than_90_days_becomes_warning` (lint.rs:758) → PASS. `entry.added_date` Display outputs `"2026-01-01"`, message assertion `contains("2026-01-01")` satisfied.
- `bin_exits_two_on_malformed_quarantine` (integration test) → PASS. Input is a 3-col line, fails at column-count check before reaching added_date parse — unchanged behavior.

- [ ] **Step 9: Commit the migration**

Run:
```bash
git add crates/raikiri-wpt/src/expectations.rs crates/raikiri-wpt/src/lint.rs
git commit -m "$(cat <<'EOF'
refactor(raikiri-wpt): QuarantineEntry.added_date を time::Date に昇格

Quarantine::parse で YYYY-MM-DD 形式検証を追加し、malformed date は
ExpectError::MalformedLine として parse 側で surfacing。
lint::detect_expired の重複した Date::parse ブランチを削除し、
90-day 閾値の比較のみに責務を集約。

- QuarantineEntry.added_date: String → time::Date
- Quarantine::parse に format_description!("[year]-[month]-[day]") で
  compile-time format literal での date parse を追加
- lint::detect_expired: Date::parse ブランチ削除、Duration インポート維持
- test: quarantine_rejects_malformed_added_date (parser-level) 追加
- test: quarantine_parses_8_cols_and_enum_values を date! macro で更新
- 既存 lint テスト (malformed_added_date_becomes_malformed_lint_issue,
  expired_quarantine_older_than_90_days_becomes_warning) は message shape
  互換のため無変更で pass
- expectations/quarantine.txt は fmt 互換で無変更 (AC #6)

Closes raikiri-spike-md0
EOF
)"
```

---

## Task 2: Format + Clippy + Final Verification

**Files:**
- No source changes expected unless fmt/clippy find issues; if so, fixes apply to `crates/raikiri-wpt/src/{expectations,lint}.rs`.

**Interfaces:**
- Consumes: The compiled state from Task 1.
- Produces: Clean-gate confirmation for md0 close.

- [ ] **Step 1: `cargo fmt --check` on raikiri-wpt**

Run: `cargo fmt --check -p raikiri-wpt`
Expected: exit 0, no diff. If any diff:
1. Run `cargo fmt -p raikiri-wpt` to apply.
2. `git add crates/raikiri-wpt/src/` and add a follow-up commit `style(raikiri-wpt): cargo fmt for md0 migration`.

- [ ] **Step 2: `cargo clippy` on raikiri-wpt**

Run: `cargo clippy -p raikiri-wpt --all-targets -- -D warnings`
Expected: exit 0, no warnings. If clippy flags something specific to the migration (e.g. `format!` inefficiency, or a lint from the newly introduced pattern):
1. Fix the flagged code.
2. Re-run clippy to verify clean.
3. If a commit was needed, include the fix in `Step 1` follow-up commit (squash) or add a separate `style(raikiri-wpt): clippy for md0 migration` commit.

- [ ] **Step 3: Final test suite re-run**

Run: `cargo test -p raikiri-wpt 2>&1 | tail -20`
Expected — the tail shows: unit **35 pass / 0 fail**, bin integration **3 pass / 0 fail**, doc-tests **1 pass / 0 fail**. AC #4-#5 both satisfied.

- [ ] **Step 4: `git status` sanity check**

Run: `git status`
Expected: clean tree on branch `worktree-raikiri-spike-md0`, with 1 (or 2 if fmt/clippy needed a follow-up) commit ahead of `main` per `git log --oneline main..HEAD`. AC #6 verification (no touch to `expectations/quarantine.txt`) confirmed by absence of that path in the diff.

- [ ] **Step 5: Ready for the finishing-a-development-branch skill to decide merge strategy**

At this point, all 6 acceptance criteria from md0 are satisfied:

| AC | Verification |
|---|---|
| 1. `added_date: time::Date`, String not retained | grep `added_date` in `crates/raikiri-wpt/src/expectations.rs` returns `Date` only |
| 2. `Quarantine::parse` raises `MalformedLine` on bad date | Task 1 Step 8: `quarantine_rejects_malformed_added_date` passes |
| 3. `lint::detect_expired` has no `Date::parse` branch | Task 1 Step 6: function is 20 lines shorter; Malformed detection is delegated |
| 4. `cargo test -p raikiri-wpt` all pass | Task 2 Step 3: 35 + 3 + 1 |
| 5. `cargo fmt --check` + `cargo clippy … -D warnings` clean | Task 2 Steps 1, 2 |
| 6. `expectations/quarantine.txt` unchanged | Task 2 Step 4: `git status` confirms |

Continue with `superpowers:finishing-a-development-branch` in the outer session.

---

## Self-Review Notes

**Spec coverage:**
- md0 Design §1 (field type) → Task 1 Step 3
- md0 Design §2 (Quarantine::parse YYYY-MM-DD validation) → Task 1 Steps 1, 4
- md0 Design §3 (quarantine.txt fmt-compatible) → Global Constraint + Task 2 Step 4
- md0 Design §4 (lint.rs Date::parse branch removal) → Task 1 Step 6
- md0 Design §5 (malformed_added_date_becomes_malformed_lint_issue retention) → Task 1 Step 8 explains message-shape preservation

**Placeholder scan:** No TBD / TODO / "add appropriate…" / "similar to Task N" — all code is fully specified.

**Type consistency:**
- `time::Date` used consistently across expectations.rs field, parse expression, lint.rs `detect_expired` `now` parameter, and lint tests (unchanged `time::macros::date!(2026 - 07 - 16)`).
- `format_description!` macro imported in expectations.rs only (added new); removed from lint.rs (no longer needed since date already parsed).
- `time::Duration::days(90)` still used in lint.rs.
- Field access: `entry.added_date` — now `time::Date`, Display gives ISO 8601, so all existing `format!("{}...", entry.added_date, ...)` sites work unchanged.
