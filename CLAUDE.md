# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## docs/superpowers/ は flow 情報

`docs/superpowers/` 配下 (plans / retros / specs / sprints) は **flow 情報であって stock
情報ではない**ため、**tracked にしない**。新規 doc も untracked のままにし、`git add -f`
で追加しないこと。規則の正典と経緯は **AGENTS.md の「docs/superpowers/ は flow 情報
(tracked にしない)」節**を参照。

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

## 使い捨て worktree は `$HOME` 配下に作る (`/tmp` に作らない)

一時的な目的 (baseline 比較、使い捨て実験など) で切る throwaway/scratch な
`git worktree` で `cargo build` / `cargo test` / `cargo bench` を走らせる場合、
その worktree は **`$HOME` 配下に作る。`/tmp` の下に作らない**。

本 repo の `/tmp` は小さな tmpfs で、**動作中の全 session が同時に共有する**
(worktree-per-task 運用のため、session 数は並行 task 数に比例して増える)。
枯渇の被害は gate の false-FAIL (bd raikiri-spike-weky) だけではない —
**Bash tool 自体が無反応になり、診断可能な error が一切出ない**状態になりうる
(bd raikiri-spike-weky への 2026-08-01 03:12 コメント記録、dz8t 実装 agent の副次的発見、
sprint/coord/7 = global Sprint 38)。

`scripts/lib/tmpdir.sh` の `TMPDIR` pin (bd raikiri-spike-weky) は
**gate script の呼び出しに限定した、既に landing 済みのより狭い緩和策**である。
本節はそれとは別に、gate 由来かどうかに関わらず **あらゆる scratch worktree に
適用される repo 全体の規約**。

## Build & Test

_Add your build and test commands here_

```bash
# Example:
# npm install
# npm test
```

## Architecture Overview

_Add a brief overview of your project architecture_

## Conventions & Patterns

Topic-scoped guides under `.claude/rules/` (read on demand, not loaded into context by default):

- `.claude/rules/no-internal-jargon-in-source-comments.md` — checked-in source comments under `crates/**` (Rust doc comments, UA CSS, etc.) must not reference bd issue IDs, milestone/epic labels, internal audit/workflow process names, or agent-memory paths; keep spec citations and technical rationale intact.
