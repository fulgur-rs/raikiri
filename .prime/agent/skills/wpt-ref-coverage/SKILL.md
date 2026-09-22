---
name: wpt-ref-coverage
description: Surveys and iteratively improves Raikiri's WPT visual reftest coverage by selecting one category/theme, implementing support, proving PASS, opening a PR, and continuing after merge. Use when asked to survey WPT reftests, choose a coverage slice, or run the WPT coverage improvement loop. Does not handle meta-assert-only tests.
---

# WPT reftest coverage loop

Use this workflow only for visual WPT reftests with `rel=match` or
`rel=mismatch`. Keep meta-assert-only tests and
`expectations/meta-assert-baseline.txt` out of scope.

## Operating contract

- Use `bd` for durable work. Work one category/theme per issue and PR.
- Use the pinned WPT checkout, bundled WPT fonts, and exact 800x600 CSS-pixel
  rendering. Record the WPT revision used by the survey and test run.
- Treat the survey as an inventory only. It does not run tests and does not
  establish a PASS. A test file with multiple references is eligible for the
  visual baseline only after every pair passes.
- Never add a failing, skipped, unrenderable, or unverified test to
  `expectations/raikiri-baseline.txt`. Never remove an existing baseline entry
  to make a run green. Baseline additions/removals follow the review rule in
  the file header; do not bypass required reviewers.
- Keep the user's existing changes untouched. Use a dedicated
  `.worktrees/<issue-or-slug>/` worktree; do not create scratch worktrees in
  `/tmp` or `$HOME`.
- The user must explicitly ask to run the end-to-end PR loop before making
  remote changes. A full-loop request authorizes a task branch, push, and PR
  for that bounded slice. Merge or enable auto-merge only after required CI and
  every required review have passed. Never bypass branch protection or merge
  with a failing/pending check. If the request is survey-only, do not edit,
  commit, push, or open a PR.
- Stop and report if the selected work crosses a project boundary, needs a
  human spec decision, requires changing visual truth, has a security concern,
  or lacks required CI/review/remote access. Do not pick the next theme until
  the current PR has merged.

## 1. Preflight and survey

1. Read `AGENTS.md`, run `bd prime`, inspect `git status`, and inspect the exact
   contents of this skill from the repository working tree. Do not overwrite
   uncommitted files.
2. Fetch the pinned WPT checkout (from the task worktree so its `target/wpt`
   link is installed):

   ```sh
   scripts/wpt/fetch.sh
   ```

3. Generate a category/theme inventory. The JSON output is the source for
   selecting test IDs; text mode is for quick review.

   ```sh
   python3 scripts/wpt/survey_reftests.py \
       --wpt-root target/wpt \
       --baseline expectations/raikiri-baseline.txt \
       --format json \
       --output target/wpt-reftest-survey.json
   python3 scripts/wpt/survey_reftests.py \
       --wpt-root target/wpt \
       --baseline expectations/raikiri-baseline.txt \
       --category css/css-text --theme hyphens --list-tests
   ```

4. Pick one unbaselined theme with local references, a supported file extension,
   and no survey blockers. Inspect its actual tests and references before
   deciding it is one semantic theme. Directory and filename-prefix grouping
   are only hints; review `theme_source` and manually group root-level files.
   If a category is absent, check the sparse patterns and fixed roots in
   `subset.txt` before concluding it has no reftests. Do not edit the shared
   sparse set for one theme. If a test uses an extension the current runner
   does not discover (for example
   `.xht`), do not claim it is runnable until the runner supports it.
5. Check `bd ready` and `bd search` for an existing issue. Reuse a suitable
   issue; otherwise create one under the WPT coverage umbrella and claim it.
   Keep the issue scope to this theme and include the test IDs, WPT revision,
   exact viewport, and acceptance criteria.

## 2. Work one theme

1. Create the dedicated worktree under `.worktrees/`. Confirm its status is
   clean before editing. Do not change `scripts/wpt/subset.txt` for a theme:
   `css/`, `fonts/`, and `images/` are stable shared roots. `fetch.sh` validates
   them and locks the shared sparse file. A stale branch's old fetch script
   fails instead of hiding tests; update the worktree from current main before
   retrying. If a selected test needs files outside those roots, stop and request
   a shared checkout-scope change; do not mutate the
   sparse set for this PR.
2. Establish the pre-change result for the selected tests. The survey itself is
   not an execution command. Use or add `raikiri-wpt` integration tests that
   run the selected WPT pairs at 800x600 with the bundled WPT font context.
   Ensure all reference links from each selected test file are exercised.
   Use image-resource loading when the selected tests require local images.
3. Implement only the smallest feature slice that can make this theme pass.
   Do not fold unrelated categories or neighboring themes into the PR.
4. Re-run the focused reftest integration test(s). Add only test IDs whose
   complete reference set now passes at the fixed context to
   `expectations/raikiri-baseline.txt`. Record failures and blockers in the
   issue; do not pin them.
5. Validate expectations and regenerate the generated coverage dashboard if
   the baseline changed:

   ```sh
   cargo run --locked -p raikiri-wpt --bin validate-expectations
   scripts/wpt/update-dashboard.sh
   ```

6. Run the applicable project gate from `AGENTS.md` and the repository's
   required checks. Do not interpret an empty/ignored test run as a pass.

## 3. PR, merge, and continue

1. Before creating a PR, confirm the diff contains only the selected theme,
   its required infrastructure/assets, and generated baseline/dashboard data.
   Report the exact PASS IDs and test command in the PR description.
2. Create the PR only for the scoped issue. Wait for all required CI checks and
   reviews. If an automatic merge is available, enable it without bypassing
   policy. Otherwise stop and hand off the PR link and outstanding requirements.
3. After the PR is merged, update/close the Beads issue only when its
   acceptance criteria and gate are complete. Refresh the survey at the same
   WPT pin and verify the new baseline membership.
4. Remove the completed worktree with `git worktree remove` and delete its
   branch only after confirming it is merged and clean. Preserve any unrelated
   worktree or user changes.
5. Continue with the next eligible category/theme only when the user asked for
   a repeated loop. Stop when the requested cycle limit is reached, no suitable
   theme remains, a blocker/human decision appears, required CI/review is not
   available, or the user asks to stop. Send a short progress update between
   PR cycles.

## Survey output interpretation

`structurally_runnable_unbaselined` means only that the survey found local
references and an extension scanned by the current tree walker. It does not
mean the renderer supports the test. Confirm support assets, fonts, parser
behavior, and every match/mismatch pair by running the actual integration
test. The survey never edits a baseline.
