# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote — separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [SYNC_CONCEPTS.md](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

## docs/superpowers/ は flow 情報 (tracked にしない)

**`docs/superpowers/` 配下 (plans / retros / specs / sprints) は flow 情報であって
stock 情報ではない** (user 明言 2026-07-26)。したがって **git で tracked にしない**。

- **既存 doc**: tracked にしない。`.gitignore` の `docs/superpowers/` で除外済み
- **新規 doc**: 同様に untracked のまま。**`git add -f` で追加しないこと**
- **例外を作らない**: 「この設計文書は重要だから stock 扱い」といった個別判断はしない。
  重要な決定は doc ではなく **bd issue / bd decision** に残す (そちらが stock)

### 経緯

2026-07-26 時点で 47 file (plans 31 / retros 9 / specs 7) が tracked のまま残っており、
しかも境界が日付順ですらなかった (`.git/info/exclude` に追加された後も一部が index に
残存)。方針と実態が食い違い、実際に retro-facilitator の誤認 (n=1 一般化 miscite) を
誘発した実績がある。

2026-07-27 に user 判断 (**Option A: 全部 untrack**) で既存 47 file を
`git rm --cached` により index から外した (bd `raikiri-spike-ggxj`)。
specs/ の 082k 設計文書と retro 9 本が untracked になる点も user 確認済み。

除外は元々 `.git/info/exclude` にあったが、**これは local 設定で共有されない**ため
`.gitignore` へ移した。`.git/info/exclude` 側の記述は残っていても害はない。

### 観測上の注意

`git worktree` は **tracked file しか materialize しない**。`docs/superpowers/` は
untracked なので、**これ以降に作成した worktree にはこのディレクトリが現れない**。
これは git の挙動であって、特定の環境の状態には依存しない。

したがって worktree 内の `ls docs/superpowers/` が空であることは、**tracking 状態の観測**
であって **実体の所在の観測ではない**。ここから「他の場所にも無い」とも「他の場所には
ある」とも**推論できない**。

実体が存在するかは環境ごとに異なる (`git clean`、untrack 前から存在する clone や worktree、
手動コピー、CI cache 等で変わる) ため、**この文書では所在を断定しない**。
必要なら **確認したい場所で直接 `ls` すること**。過去の内容は履歴から読める:

```bash
P=docs/superpowers/specs/<name>.md
git show "$(git rev-list -1 HEAD -- "$P")^:$P"   # untrack 直前の内容を読む
```

末尾の `^` が要る。`git rev-list -1 -- <path>` が返すのは **untrack commit 自身**で、
そこには既に path が無いため、`^` を付けないと
`fatal: path ... exists on disk, but not in <sha>` になる (2026-07-27 に実測)。

**working tree に無くても履歴には残る**という点は、`docs/superpowers/specs/...` を参照する
doc comment にも当てはまる。2026-07-27 時点で **7 file 中 8 箇所** (`crates/` 6 file +
`docs/feasibility-report.md`) がこれらの spec を参照しているが、**参照先が working tree に
無い環境では dangling ref になる**。これは Option A の既知の帰結で、読みたい場合は
上の履歴参照を使う。

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

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
