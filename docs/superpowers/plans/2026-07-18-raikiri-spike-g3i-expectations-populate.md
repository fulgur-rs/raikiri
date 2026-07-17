# raikiri-wpt expectations/ populate (g3i) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Populate `expectations/tracked-wpt.txt` and `expectations/known-issues.txt` with spec §12.9 categories + §2 Non-Goals waivers, rewrite `raikiri-baseline.txt` / `quarantine.txt` / `deprecated.txt` headers with populate-policy documentation, and strengthen the `ExpectationSet::load_from_workspace_root` doctest to assert the three runner-generated files remain empty.

**Architecture:** Text-file edits under `expectations/` + one Rust doctest edit under `crates/raikiri-wpt/src/expectations.rs`. No parser or lint changes. Verification via existing `cargo test -p raikiri-wpt` (unit + doctest + integration) and the `validate-expectations` bin, plus workspace-wide `cargo build`. Populated files parse via existing `TrackedWpt::parse` / `KnownIssues::parse` (both accept free-form patterns).

**Tech Stack:** Cargo workspace, Rust 2024 edition, `raikiri-wpt` crate (parsers + lint + validate-expectations bin).

## Global Constraints

- **UTF-8 encoding** for all `expectations/*.txt` (§12.10 validation).
- **No trailing whitespace** on data lines (`validate-expectations` fails on it, §12.10).
- **Comment lines** start with `#`; empty lines are ignored (parser contract in `expectations.rs`).
- **tracked-wpt.txt / known-issues.txt semantics** — trailing `/` on an entry means "dir prefix, `starts_with` match at runtime"; no trailing `/` means "exact test_id".
- **known-issues.txt data format** — `<test_id_or_pattern> | <reason>` (`|`-separated, both sides trimmed and non-empty).
- **raikiri-baseline.txt / quarantine.txt / deprecated.txt** — must remain header-only after this plan.
- **Branch** — all commits go on the current worktree branch (`worktree-g3i-expectations-populate`).
- **Commit style** — follow recent workspace commits (title `<type>(<scope>): <summary>`; body with a short "why").

---

### Task 1: Populate `expectations/tracked-wpt.txt`

**Files:**
- Modify: `expectations/tracked-wpt.txt` (currently header-only, 7 lines)

**Interfaces:**
- Consumes: existing `TrackedWpt::parse` (accepts free-form `Vec<String>`, no format constraint).
- Produces: 14 tracked entries visible via `ExpectationSet::load_from_workspace_root().tracked.entries`; consumed by M3 runner (out of scope for this plan).

- [ ] **Step 1: Overwrite `expectations/tracked-wpt.txt`**

Replace the entire file contents with the block below.

```
# expectations/tracked-wpt.txt — WPT categories under tracking (spec §12.9).
#
# Format: one WPT test_id or category path prefix per line.
# Lines starting with '#' are comments. Empty lines are ignored.
#
# Semantics (consumed by M3 runner):
# - Entries ending with '/' are directory prefixes; runner will match any
#   test_id whose path starts with the prefix.
# - Entries not ending with '/' are treated as exact test_id matches.
# Tracked tests emit as T3 informational only; regression does NOT block PR.
#
# Priority reflects raikiri's *scratch-build dependency order*, not project
# end-goal. css-page / css-fragmentation are the project goal but come AFTER
# the foundation stabilises.

# ── P1 — Foundation (must work first; nothing renders without these) ──
css/css-fonts/
css/css-color/
css/css-backgrounds/
css/css-values/
css/css-text/
css/css-writing-modes/
css/selectors/
html/rendering/

# ── P2 — Layout primitives (build on foundation) ──
css/css-tables/
css/css-grid/
css/css-flexbox/

# ── P3 — GCPM / paged media (raikiri's project goal, layered on P1-P2) ──
css/css-page/
css/css-fragmentation/

# ── P4 — Low priority ──
css/css-transforms/
```

- [ ] **Step 2: Run validate-expectations bin**

Run: `cargo run -p raikiri-wpt --bin validate-expectations`
Expected exit code: `0`. Expected stdout ends with `expectations: clean`.

If exit is non-zero, the most likely cause is trailing whitespace on a data line or non-UTF-8 encoding — inspect the file with `cat -A expectations/tracked-wpt.txt | grep '\$'` (each visible `$` is a line ending; look for `^I` tabs or `<space>+\$` patterns before them).

- [ ] **Step 3: Run existing raikiri-wpt tests**

Run: `cargo test -p raikiri-wpt --lib`
Expected: all 39 unit tests still `ok`. (`TrackedWpt::parse` accepts free-form strings, so populated content does not break parser tests.)

- [ ] **Step 4: Commit**

```bash
git add expectations/tracked-wpt.txt
git commit -m "$(cat <<'EOF'
feat(raikiri-wpt): populate tracked-wpt.txt with §12.9 + blitz-overlap categories (g3i)

14 entries structured as P1 Foundation (fonts/color/backgrounds/values/text/
writing-modes/selectors/html-rendering), P2 Layout primitives (tables/grid/
flexbox, blitz overlap), P3 GCPM (css-page/fragmentation), P4 transforms.
Priority reflects scratch-build dependency order, not project end-goal.

Semantics: trailing '/' = dir prefix (M3 runner starts_with match).

Refs: raikiri-spike-g3i, spec §12.9.
EOF
)"
```

---

### Task 2: Populate `expectations/known-issues.txt`

**Files:**
- Modify: `expectations/known-issues.txt` (currently header-only, 7 lines)

**Interfaces:**
- Consumes: existing `KnownIssues::parse` (splits `line.splitn(2, '|')`; both sides trimmed, must be non-empty).
- Produces: 6 `(pattern, reason)` entries visible via `ExpectationSet::load_from_workspace_root().known_issues.entries`.

- [ ] **Step 1: Overwrite `expectations/known-issues.txt`**

Replace the entire file contents with the block below.

```
# expectations/known-issues.txt — WPT tests judged as "cannot pass, no practical impact"
# (spec §12.9).
#
# Format: <test_id_or_pattern> | <reason>
# Lines starting with '#' are comments. Empty lines are ignored.
#
# Semantics (consumed by M3 runner):
# - The `test_id_or_pattern` is a WPT test id or a directory prefix
#   ending in '/'; runner treats matched tests as intentionally SKIPped.
# - Results are not reported as regressions and do not count toward
#   tracked-wpt statistics.

# ── §12.9 explicit non-tracked (interactive rendering non-goal) ──
css/css-animations/ | Non-goal (interactive, §2 Non-Goals + §12.9)
css/css-transitions/ | Non-goal (interactive, §2 Non-Goals + §12.9)
html/interaction/ | Non-goal (interactive rendering, §2 Non-Goals + §12.9)

# ── §2 Non-Goals — Japanese vertical typesetting / ruby (MVP horizontal only) ──
css/css-ruby/ | Non-goal for MVP (JIS X 4051 / 縦書き outside MVP scope, §2 Non-Goals)

# ── §2 Non-Goals — Accessibility tree construction ──
accname/ | Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)
wai-aria/ | Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)
```

- [ ] **Step 2: Run validate-expectations bin to confirm well-formed**

Run: `cargo run -p raikiri-wpt --bin validate-expectations`
Expected exit code: `0`. Output ends with `expectations: clean`.

If exit is non-zero, inspect stderr. The most likely failure is a `MalformedLine` from a `|`-parse error — verify each data line has exactly one `|` with non-empty text on both sides.

- [ ] **Step 3: Confirm existing unit tests still pass**

Run: `cargo test -p raikiri-wpt --lib`
Expected: all 39 unit tests still `ok`. (No new tests added; existing parser tests already cover this format.)

- [ ] **Step 4: Commit**

```bash
git add expectations/known-issues.txt
git commit -m "$(cat <<'EOF'
feat(raikiri-wpt): populate known-issues.txt with §12.9 + §2 Non-Goals waivers (g3i)

6 entries: §12.9 explicit non-tracked (animations/transitions/html-interaction)
+ §2 Non-Goals (css-ruby for MVP; accname/wai-aria — Consumer builds a11y tree
from hints, not raikiri). Reason strings carry §-references for traceability.

Refs: raikiri-spike-g3i, spec §12.9 + §2 Non-Goals.
EOF
)"
```

---

### Task 3: Rewrite `raikiri-baseline.txt` / `quarantine.txt` / `deprecated.txt` headers

**Files:**
- Modify: `expectations/raikiri-baseline.txt`
- Modify: `expectations/quarantine.txt`
- Modify: `expectations/deprecated.txt`

**Interfaces:**
- Consumes: existing `Baseline::parse` / `Quarantine::parse` / `Deprecated::parse` (comment-only files parse to empty structures).
- Produces: files remain header-only (no data lines added), but headers now document populate policy.

- [ ] **Step 1: Overwrite `expectations/raikiri-baseline.txt`**

Replace the entire file contents with the block below.

```
# expectations/raikiri-baseline.txt — T2 gate: WPT tests raikiri currently passes
# (spec §12.10). Regression on these blocks PR merge.
#
# Format: one test_id per line. Lines starting with '#' are comments.
# Empty lines are ignored.
#
# ── Populate policy ─────────────────────────────────────────────────
# This file is *runner-generated*, NOT hand-authored:
# 1. At M3 kickoff (once the reftest runner is operational), run the runner
#    over ALL executable WPT tests — every test NOT excluded by
#    known-issues.txt or deprecated.txt.
#    Note: quarantine.txt entries are NOT excluded from the baseline.
#    Quarantine filters are platform-aware (§12.10 8-col format), so a
#    test flaky only on some (platform, arch, renderer, tolerance) tuples
#    must still receive baseline regression protection on stable tuples.
#    The runner suppresses baseline gating at execution time when the
#    current tuple matches a quarantine rule (T3 informational).
# 2. Filter runner output to tests with STATUS = PASS.
# 3. Open a PR that adds them to this file. The PR is the initial baseline
#    migration and requires 2-reviewer approval per §12.10.
# Subsequent additions / removals follow the same 2-reviewer rule.
#
# Scope note: the baseline is *scope-independent* per §2 Goals ("regression
# detection の flat rule": any previously-passing test regressing blocks
# merge, whether or not the test's category is tracked). tracked-wpt.txt is
# only a T3 reporting/prioritization view of the WPT surface; it does not
# restrict which tests qualify for the baseline.
#
# Lint follow-up: the current lint (raikiri-wpt::lint::detect_conflicting)
# flags every baseline∩quarantine pair as Conflicting. Baseline↔quarantine
# co-listing is a legitimate design (per this policy); the platform-aware
# filter-overlap analysis to suppress spurious conflicts is tracked as
# raikiri-spike-a6s.
#
# See blitz's wpt/runner/src/report.rs `generate_expectations` for a
# reference implementation of the runner-generated approach.
```

- [ ] **Step 2: Overwrite `expectations/quarantine.txt`**

Replace the entire file contents with the block below.

```
# expectations/quarantine.txt — Flaky test temporary shelf (spec §12.10, platform-aware).
#
# Format: test_id | platform | arch | renderer | tolerance | reason | issue_link | added_date
#
# platform:  linux / macos / windows / *
# arch:      x86_64 / aarch64 / *
# renderer:  vello_cpu / skia / tiny_skia / *
# tolerance: pixel-exact / low / medium / high / *
#
# Multiple entries per test_id (different platform combinations) are allowed.
# Lines starting with '#' are comments. Empty lines are ignored.
#
# ── Populate policy ─────────────────────────────────────────────────
# Empty until the first flake is observed in M3+ CI runs. Additions follow
# §12.10 quarantine procedure:
#   1. Test intermittent-fails ≥3 times → developer opens quarantine PR
#   2. Add entry with reason + issue link + added_date
#   3. Weekly review; strategic review at 90-day expiration (§12.10)
```

- [ ] **Step 3: Overwrite `expectations/deprecated.txt`**

Replace the entire file contents with the block below.

```
# expectations/deprecated.txt — WPT tests fully excluded from evaluation
# (spec §12.10 precedence 1; highest priority).
#
# Format: one test_id per line. Lines starting with '#' are comments.
# Empty lines are ignored.
#
# ── Populate policy ─────────────────────────────────────────────────
# Empty until raikiri encounters crashers or tests requiring evaluation-blocking
# runner-level workarounds. Additions follow §12.10 (2-reviewer approval).
#
# Analog: blitz's `wpt/runner/src/main.rs::BLOCKED_TESTS` const slice
# (e.g., wgpu buffer overflows, ImageBuffer overflow crashers). raikiri
# keeps them in this file rather than inline Rust for auditability and
# to keep the runner rebuild-free when the exclusion list changes.
```

- [ ] **Step 4: Run validate-expectations bin**

Run: `cargo run -p raikiri-wpt --bin validate-expectations`
Expected exit code: `0`. Output ends with `expectations: clean`.

The 3 files parse to empty (`Baseline::entries.is_empty() == true`, etc.); lint's Conflicting / Duplicate / Malformed / Expired checks find nothing.

- [ ] **Step 5: Commit**

```bash
git add expectations/raikiri-baseline.txt expectations/quarantine.txt expectations/deprecated.txt
git commit -m "$(cat <<'EOF'
docs(raikiri-wpt): rewrite baseline/quarantine/deprecated headers with populate policy (g3i)

Files stay header-only. Headers now document:
- raikiri-baseline.txt: runner-generated at M3 kickoff, blitz's
  generate_expectations as reference for the approach.
- quarantine.txt: §12.10 quarantine procedure (3-fail threshold, weekly
  review, 90-day strategic review).
- deprecated.txt: analog of blitz's BLOCKED_TESTS; kept file-side (not
  inline Rust) for auditability and rebuild-free updates.

Refs: raikiri-spike-g3i, spec §12.10.
EOF
)"
```

---

### Task 4: Strengthen `ExpectationSet::load_from_workspace_root` doctest

**Files:**
- Modify: `crates/raikiri-wpt/src/expectations.rs` (doctest at line 26-30)

**Interfaces:**
- Consumes: existing `ExpectationSet` public fields (`baseline`, `quarantine.entries`, `deprecated.entries`).
- Produces: doctest asserts the empty-contract for the three runner-generated files, providing a red-signal when M3 kickoff populates `baseline`.

- [ ] **Step 1: Locate the current doctest**

Read `crates/raikiri-wpt/src/expectations.rs` lines 22-31. Current content:

```rust
/// All expectations files combined.
///
/// ```no_run
/// use raikiri_wpt::expectations::ExpectationSet;
/// let set = ExpectationSet::load_from_workspace_root().unwrap();
/// assert!(set.baseline.is_empty()); // M1 skeleton: workspace files are header-only
/// ```
#[non_exhaustive]
pub struct ExpectationSet {
```

- [ ] **Step 2: Replace the doctest block**

Change the doctest inside the `///` block from:

```rust
/// ```no_run
/// use raikiri_wpt::expectations::ExpectationSet;
/// let set = ExpectationSet::load_from_workspace_root().unwrap();
/// assert!(set.baseline.is_empty()); // M1 skeleton: workspace files are header-only
/// ```
```

to:

```rust
/// ```no_run
/// use raikiri_wpt::expectations::ExpectationSet;
/// let set = ExpectationSet::load_from_workspace_root().unwrap();
/// // baseline/quarantine/deprecated are runner-generated at M3 kickoff and
/// // remain empty until then; tracked/known_issues are populated statically
/// // from spec §12.9 (see expectations/*.txt).
/// assert!(set.baseline.is_empty());
/// assert!(set.quarantine.entries.is_empty());
/// assert!(set.deprecated.entries.is_empty());
/// ```
```

Use the Edit tool. The `old_string` must include the entire block above (5 lines from ` /// ```no_run` through ` /// ``` `) so it uniquely matches.

- [ ] **Step 3: Run the doctest**

Run: `cargo test -p raikiri-wpt --doc`

Expected: doctest compile passes (it is `no_run` so it does not execute — but the compile check verifies field access is valid: `set.baseline.is_empty()`, `set.quarantine.entries.is_empty()`, `set.deprecated.entries.is_empty()`).

The relevant types (verified against `expectations.rs`):
- `Baseline` has method `is_empty()` (line 211-213).
- `Quarantine::entries: Vec<QuarantineEntry>` is `pub` (line 256); `Vec::is_empty()` is a std method.
- `Deprecated::entries: HashSet<String>` is `pub` (line 224); `HashSet::is_empty()` is a std method.

If the doctest fails to compile, verify the field visibility hasn't drifted from the read above.

- [ ] **Step 4: Run the full raikiri-wpt test suite**

Run: `cargo test -p raikiri-wpt`
Expected: all 39 unit tests + 3 integration tests + doctest pass. No new failures.

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-wpt/src/expectations.rs
git commit -m "$(cat <<'EOF'
test(raikiri-wpt): strengthen ExpectationSet doctest with baseline/quarantine/deprecated empty contract (g3i)

The three runner-generated files must remain empty until M3 kickoff.
Adding assertions gives us a red-signal when the baseline PR
lands and the doctest then needs a coordinated update.

Refs: raikiri-spike-g3i.
EOF
)"
```

---

### Task 5: Workspace verification + follow-up beads issues

**Files:**
- No file edits.
- Beads: create 4 follow-up issues via `bd create`.

**Interfaces:**
- Consumes: `bd` CLI, `cargo` workspace build.
- Produces: 4 new open beads issues + green workspace build attesting no regression.

- [ ] **Step 1: Full raikiri-wpt suite**

Run: `cargo test -p raikiri-wpt`
Expected: all pass (unit + integration + doctest). Same green as Task 4.

- [ ] **Step 2: validate-expectations bin end-to-end**

Run: `cargo run -p raikiri-wpt --bin validate-expectations`
Expected exit code: `0`. Last line: `expectations: clean`.

- [ ] **Step 3: Workspace build**

Run: `cargo build --workspace`
Expected: `Finished ... in ...s` with no warnings introduced by this branch. If warnings appear that are not present on `origin/main` for the same files, investigate.

- [ ] **Step 4: File follow-up beads issue — spec §12.9 revision**

Run:

```bash
bd create "spec §12.9 revision: split tracked-priority into project-priority × scratch-priority axes" \
  -t task -p 3 \
  --description "$(cat <<'EOF'
Follow-up from raikiri-spike-g3i. tracked-wpt.txt is now populated with a
priority that reflects scratch-build dependency order (P1 Foundation → P2
Layout primitives → P3 GCPM/paged-media → P4 transforms), which diverges
from spec §12.9's project-priority framing (paged media as top priority).

Revise §12.9 table to make both axes explicit — or consolidate to
scratch-priority if the project-priority framing is judged redundant
with §2 Goals. Coordinate with raikiri-spike-m1.19 (m0-drift-spec-revise).
EOF
)"
```

Expected: prints a new issue ID; note it for later.

- [ ] **Step 5: File follow-up beads issue — css-fonts scope**

Run:

```bash
bd create "css-fonts scope decision: web-font (@font-face remote) M1/M2/M3 placement" \
  -t task -p 3 \
  --description "$(cat <<'EOF'
Follow-up from raikiri-spike-g3i. tracked-wpt.txt now includes
css/css-fonts/ as a P1 Foundation category, but WPT css-fonts tests
routinely depend on @font-face remote-loaded fonts. Depending on when
raikiri wires web-font loading (SandboxedNetProvider path), a large
subset of css-fonts WPT tests will fail until then.

Decide: does raikiri M1/M2/M3 do remote web fonts, and if not, add
specific test-id patterns to known-issues.txt to keep tracked-wpt.txt
signal clean.
EOF
)"
```

- [ ] **Step 6: File follow-up beads issue — baseline runner-generated populate**

Run:

```bash
bd create "raikiri-baseline.txt: runner-generated populate at M3 kickoff" \
  -t task -p 2 \
  --description "$(cat <<'EOF'
Follow-up from raikiri-spike-g3i. raikiri-baseline.txt header now
documents that the file is *runner-generated*: at M3 kickoff, run the
reftest runner over ALL executable WPT tests (any test not in
known-issues or deprecated), filter to STATUS=PASS, and open the
initial baseline PR (2-reviewer approval per §12.10).

Scope is intentionally flat per §2 Goals — the baseline is not
restricted to tracked-wpt.txt categories. Quarantine entries are NOT
excluded from the baseline: quarantine filters are platform-aware
(§12.10 8-col format), so a test flaky on some tuples must still
receive baseline protection on stable tuples. The runner suppresses
baseline gating at execution time when the current (platform, arch,
renderer, tolerance) matches a quarantine rule.

Lint follow-up: raikiri-spike-a6s tracks tuning
raikiri-wpt::lint::detect_conflicting to suppress spurious
baseline∩quarantine conflicts via filter-overlap analysis.

Ref implementation: blitz's wpt/runner/src/report.rs::generate_expectations.

Blocked on M3 reftest runner (raikiri-spike-m1.* → M3 issues).
EOF
)"
```

- [ ] **Step 7: File follow-up beads issue — a11y tree consumer boundary**

Run:

```bash
bd create "a11y tree boundary verification: hint contract with fulgur at M3 kickoff" \
  -t task -p 3 \
  --description "$(cat <<'EOF'
Follow-up from raikiri-spike-g3i. known-issues.txt now waives accname/
and wai-aria/ WPT categories as "Consumer builds a11y tree from hints"
per §2 Non-Goals. Confirm at M3 kickoff that the hint contract
(bookmark_hints / heading_structure / structural_hints) actually
supports fulgur constructing a compliant a11y tree — or revise
known-issues.txt / §2 boundary if the hint surface is insufficient.
EOF
)"
```

- [ ] **Step 8: Final status check**

Run: `git status && git log --oneline -8`
Expected:
- Working tree clean (all edits committed by prior tasks).
- Last 4 commits on the branch:
  ```
  <hash> test(raikiri-wpt): strengthen ExpectationSet doctest with baseline/quarantine/deprecated empty contract (g3i)
  <hash> docs(raikiri-wpt): rewrite baseline/quarantine/deprecated headers with populate policy (g3i)
  <hash> feat(raikiri-wpt): populate known-issues.txt with §12.9 + §2 Non-Goals waivers (g3i)
  <hash> feat(raikiri-wpt): populate tracked-wpt.txt with §12.9 + blitz-overlap categories (g3i)
  ```

No commit is created in this task itself — the 4 `bd create` calls are external state changes that beads persists on its own (Dolt DB + `.beads/issues.jsonl` export).
