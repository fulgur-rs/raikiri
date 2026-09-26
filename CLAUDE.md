# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## docs/superpowers/ は flow 情報

`docs/superpowers/` 配下 (plans / retros / specs / sprints) は **flow 情報であって stock
情報ではない**ため、**tracked にしない**。新規 doc も untracked のままにし、`git add -f`
で追加しないこと。規則の正典は **AGENTS.md の「docs/superpowers/ は flow 情報
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

## loopback を使う Rust test は sandbox 外で実行する

`raikiri-net` の `http-ureq` suite と `raikiri-wpt` の browser test は loopback
listener を bind する。network 制限付き sandbox 内では `PermissionDenied` になり、
実装とは無関係な false failure になるため、この2つの `cargo test` は最初から sandbox
外で実行すること。Codex向けの command-specific な許可は
`.codex/rules/cargo-test.rules` に定義している。testをskipしたり
`PermissionDenied`を成功扱いにしたりしない。

## 使い捨て worktree は `.worktrees/` 配下に作る (`/tmp` に作らない)

一時的な目的 (baseline 比較、使い捨て実験など) で切る throwaway/scratch な
`git worktree` で `cargo build` / `cargo test` / `cargo bench` を走らせる場合、
その worktree は **repo 直下の `.worktrees/<name>/` に作る。
`/tmp` の下にも `$HOME` 直下にも作らない**。
規則の正典は **AGENTS.md の同名節**を参照。

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
