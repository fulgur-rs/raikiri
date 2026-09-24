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
stock 情報ではない**。したがって **git で tracked にしない**。

- **既存 doc**: tracked にしない。`.gitignore` の `docs/superpowers/` で除外済み
- **新規 doc**: 同様に untracked のまま。**`git add -f` で追加しないこと**
- **例外を作らない**: 「この設計文書は重要だから stock 扱い」といった個別判断はしない。
  重要な決定は doc ではなく **bd issue / bd decision** に残す (そちらが stock)

## `crate::…` pointer は intra-doc link で書く

**doc comment (`///` / `//!`) 中の `crate::…` 参照は intra-doc link
(``[`crate::foo::Bar`]``) で書く。plain code span (``` `crate::foo::Bar` ```) で書かない。**
module-relative な pointer (``` `cascade::foo` ```) も同じ — audit grep を `crate::` だけで
書くとこの形を取りこぼす。plain code span の pointer は path が誤っていても検出されない —
rustdoc は intra-doc link 記法しか解決検証しないため。

### 規約

1. **既定は link 化**。
2. **crate-internal な target を指してよい。** その crate の `lib.rs` (または link を書く
   側の module の先頭) に `#![allow(rustdoc::private_intra_doc_links)]` を宣言する。
   この allow が黙らせるのは「private を指した」という**表示上**の lint だけで、
   `broken_intra_doc_links` はそのまま残る (本 repo の doc command はすべて `-D warnings`
   を渡すので hard error) ため **path の正しさの検証は失われない**。
   宣言の有無は `git grep -n 'private_intra_doc_links' -- crates/` で確認する。
   **hit の中身まで見ること** — この grep は (a) `lib.rs` の crate 全体宣言、
   (b) 単一 module file 先頭の module scope 宣言、(c) lint 名に言及しただけの comment を
   区別しない。**(b) が何を covers するかは grep からは予測できない** — 同じ
   target・同じ (b) でも、referrer の書き方だけで黙るか red になるかが反転する
   実測を確認済み。分類せず「わざと壊して確かめる」で個別に決めること。
3. **module-private な item を指すときは、既定では visibility を広げない。**
   ``[`mod@crate::cascade`] の `collect_cascaded` `` のように **module だけ link 化し
   item 名は plain code span** にする。**この形も module 自身が link 元から見えなければ
   解決しない** — 迷ったら下の「わざと壊して確かめる」で決めること。
   `pub(crate)` へ広げるのは他 module の doc から item 自体を指す必要がある場合に限り、
   **広げた item の doc に理由を 1 行書く** (例: `crates/raikiri-style/src/cascade.rs` の
   `apply_value`)。

   **hatch の閾値**: 「widening が gate で enforce されるなら可、補助 command
   (`--document-private-items`) でしか enforce されないなら本項の `mod@` 代替に倒す」。
   判定 command は上の「わざと壊して確かめる」手順そのまま (対象 item を 1 つだけ
   private に戻し、gate command で red になるかを見る) — 追加の道具は要らない。
   gate §8.1 の doc build には `--document-private-items` を足していないため、
   補助 command でしか検証されない widening は検証利得ゼロで visibility 拡大だけが
   残る (足していない理由は後述「既知の限界」参照)。
4. **同じ scope に同名の item がある module は `mod@` を付ける。** `raikiri-style` では
   `pub mod cascade` と `pub use cascade::{…, cascade}` が module と関数を同名で crate root
   に置くため ``[`mod@crate::cascade`]`` と書く。無印は ambiguous link になる。
5. **trait impl の method は trait 側の full path で書く** —
   ``[`cssparser::DeclarationParser::parse_value`]``。trait を宣言している crate からの path
   なら import 無しでも依存 crate のものでも解決する。書けない場合に限り `Type` までを
   link 化し method 名を code span で添える。

### ⚠️ 「link 化した = 検証された」ではない

gate `§8.1` の `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` が検証するのは
**rustdoc が document する doc に書かれた link だけ**である。**どの位置が検証されるかは
source の見た目から予測できない。**

反転させる変数として少なくとも **module の nest、`pub use` re-export の有無、
`#[doc(hidden)]`、`#[test]` 属性、target 自身の可視性**の 5 つが確認されており、
**網羅は取れていない**。

**したがって分類で判断しないこと。測ること。** 触る crate に対し gate とは別に 2 本走らせる:

```
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items -p <crate>
RUSTDOCFLAGS="-D warnings --cfg test" cargo doc --no-deps --document-private-items -p <crate>
```

- 判定は **baseline 差分**で行う。crate によっては既存の未修正 error が残るので
  「clean になるか」では判定できず、「自分が追加した link だけ見る」でも**不足**である
  (item の可視性を狭める / rename する変更は、**自分が触っていない別 module の link** を
  壊す)。変更前後で同じ 2 本を走らせ **`error` 行が 1 件も増えていない**ことを確認する。
- **`E0432` / `E0433` を含む run は「検証していない」と扱う。** `--cfg test` は
  `cargo doc` が dev-dependency を link しないことと組み合わさり rustdoc を compile 段階で
  止めることがあり、その run では intra-doc link 検査が 1 本も走らない
  (raikiri-vrt では既存の unresolved link が E0432 に隠れて消える)。
  **その run は判定材料から外し、もう 1 本の結果で判断すること。**
  **2 本とも `E0432` / `E0433` で止まった場合は、その crate の doc build は
  auxiliary command では判定不能** — link の正しさは gate と同じ command
  (`--document-private-items` 無し) 1 本だけで判断すること。
- **ある位置が検証されるかを知りたい場合は、link をわざと壊して**
  (``[`crate::foo::Bar`]`` → ``[`crate::foo::BarXX`]``) **どの command が自分の `file:line`
  を報告するかを見る。** 無出力は「正しい」ではなく「見られていない」かもしれない。
  gate と同じ command で出た位置だけが以後 CI に守られる。補助 command でだけ出た位置は
  authoring 時の 1 回きりの確認であり、**壊した分は必ずその場で戻すこと** — 忘れると
  gate は green のまま壊れた pointer が merge される。

### opt-out (link 化しない)

どの command でも検証できない位置は code span にする。以下は今日わかっている該当で、
**網羅である保証は無い** — 迷ったら上の「わざと壊して確かめる」で決めること。

1. **歴史参照** (既に削除された item を指す): ``` `crate::target` (removed) ``` の形で
   **code span と同じ行に** `(removed)` marker を置く (1 行 grep で audit できるように)。
   例: `crates/raikiri-dom/src/running.rs`。
2. **指す先が `#[cfg(test)] mod tests` の中**: 一般に解決しない。plain code span のままに
   する (path 中の `tests::` が test であることを示すので追加の marker は要らない)。
   例: `crates/raikiri-style/src/rule.rs`。
   ⚠️ **一部の組合せでは解決する** (link 元も同じ test module 内で、指す先が `#[test]` でなく
   十分な可視性を持つ場合)。link 化したいなら**測ってから**にすること。
3. **書いた場所が rustdoc に拾われない位置** (今日わかっている該当。**この列挙も
   網羅の保証は無い** — 上の「⚠️」で述べた反転変数がここにも及ぶので、迷ったら
   分類ではなく測って決めること): `#[test]` item の doc、関数 body の中で宣言された
   item の doc、`#[doc(hidden)]` な item (module を除く) の doc、
   `crates/*/tests/` `benches/` `examples/` (`cargo doc` が document しない)、
   `#[cfg]` で落ちる item、未展開の `macro_rules!` body。

### 既知の限界

- **plain `//` comment には rustdoc が届かない。link 化という remedy が原理的に
  適用できない。** rustdoc は `///` / `//!` しか読まない。plain `//` comment に
  intra-doc link (bracket 記法) を書いても `RUSTDOCFLAGS="-D warnings" cargo doc`
  はその byte を 1 つも見ないので、path が誤っていても永遠に検出されない —
  bracket は「検証済みに見えるのに実は未検証」でかえって bare code span より危険。
  **したがって plain `//` comment 内では、`crate::` 接頭辞の有無にかかわらず
  bracket link 構文 (`[...]` / `` [`...`] ``) を一切書かない。** 既存の
  `crate::…` pointer を含め、bare code span (`` `crate::foo::Bar` ``) に統一する。
  crate:: を含まない bracket も同様に禁止 — 危険性は crate:: の有無と無関係
  (rustdoc がその位置を 1 byte も読まないという事実は pointer の中身に依らない)。
  `scripts/doc-pointer-lint.sh` がこの blanket rule を hard-zero check として
  強制する。**強制の範囲は `crates/*/src/**/*.rs` のみ**。`crates/*/tests/`
  `benches/` `examples/` は checker の対象外 (continuously enforced ではない)。
- **既存の `#[test]` item doc に残る短縮 link** (``[`expand_shorthand_into`]`` 等)
  は opt-out 3 の位置なので未検証。**新規に書く doc では opt-out 3 に従い code
  span にすること** — この位置に新しく short-form (non-`crate::`) bracket を
  書いても `scripts/doc-pointer-lint.sh` は検知しない (short-form / non-`crate::`
  pointer はこの checker のいずれの role の対象にも入らない)。**`crate::` 接頭辞を
  伴う bracket に限っては** role 4 (informational — ratchet でも gate でもない、
  `#[cfg(test)] mod …` block 内の linked `crate::…` span を可視化するだけの role)
  が新規発生を検知して毎 run の summary に出すが、commit は止めない — 「検知」は
  できても「防止」ではないので、opt-out 3 の再発**防止**という意味では本 checker
  は依然 scope 外のまま (理由は `scripts/lib/doc_pointer_lint.py` のモジュール
  docstring 参照)。
- **本規約には enforcement 機構が今も一部無い。** 規約に従わない新規記述の一部は
  何も止めない。`scripts/doc-pointer-lint.sh` が以下の 2 点を自動 enforcement 下に
  置く:
  - plain `//` comment の bracket link (上のbulletの blanket rule) — hard-zero。
  - doc comment の bare `crate::…` pointer — pinned baseline
    (`scripts/lib/doc_pointer_lint_baseline.txt`) を超えないことを ratchet で
    強制。baseline は「今日値まで許容し、それ以上増やさない」ための pin であり、
    0 への一括削減は本 checker の scope 外。**そのための follow-up task に
    着手する場合は、着手時に bd issue として起票すること** (未起票)。

  **実行方法**: `scripts/doc-pointer-lint.sh` (追加で `-v` で全 occurrence を
  列挙、`--print-count` で ratchet 対象件数だけを出力してbaseline再生成に使う)。
  exit 0 = 両 role とも pass、1 = いずれか fail、2 = baseline file が
  読めない等の tooling error。**gate §8.1 / `scripts/gate.sh` への統合はしていない**
  — 本 checker は standalone。

  **opt-out 3 (rustdoc に拾われない位置) は本 checker では静的判定できない**。
  判定できない位置は既定で ratchet 対象に **含める** (fail-closed)。個別に
  除外したい場合のみ、`scripts/lib/patch_coverage.py` の `cov:ignore:` と同じ
  「暗黙の免除を作らない」思想で、対象行に `// doc-pointer-lint:ignore: <reason>`
  marker を書く (`cov:ignore:` と異なり同一行のみ有効 — block scoping は無い)。
  `(removed)` marker (opt-out 1) と `tests::` を含む path (opt-out 2) は
  この checker が自動で除外する。
- **toolchain 依存がある。** intra-doc link の解決は rustc version で変わる。
  `rust-toolchain.toml` の pin では出ない unresolved link が新しい toolchain では
  出る実例があるため、toolchain bump 時は本節の command を再走させること。
- **gate §8.1 の doc build は private / pub(crate) item の doc link を検証しない。**
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` (gate §8.1 が走らせる
  既定 command) は `--document-private-items` を付けないため、public item の doc
  comment 内の link しか解決しない。**意図的に gate には追加していない** — doc
  build 時間の増加と missing_docs 相当の露出面拡大というコストが、未検証リスクに
  見合わないという判断。private / pub(crate) item の doc link を検証したい場合は、
  上の「わざと壊して確かめる」節の 2 本の補助 command を個別に走らせること。

## 使い捨て worktree は `.worktrees/` 配下に作る (`/tmp` に作らない)

一時的な目的 (baseline 比較、使い捨て実験など) で切る throwaway/scratch な
`git worktree` で `cargo build` / `cargo test` / `cargo bench` を走らせる場合、
その worktree は **repo 直下の `.worktrees/<name>/` に作る。
`/tmp` の下にも `$HOME` 直下にも作らない**。

```bash
git worktree add .worktrees/<name> -b <branch>
```

`.worktrees/` は `.gitignore` 済み (OpenCode worktrees 枠) で、`git worktree list`
で一覧できる。`scripts/wpt/fetch.sh` は物理 checkout を
`$HOME/.cache/raikiri/wpt` に置き、どの worktree から呼ばれても各
`target/wpt` をそこへの symlink にする。`target/` が cleanup で消えても
checkout は残り、fetch/gate が必要な symlink を再作成する。

本 repo の `/tmp` は小さな tmpfs で、**動作中の全 session が同時に共有する**
(worktree-per-task 運用のため、session 数は並行 task 数に比例して増える)。
枯渇の被害は gate の false-FAIL だけではない — **Bash tool 自体が無反応になり、
診断可能な error が一切出ない**状態になりうる。

`scripts/lib/tmpdir.sh` の `TMPDIR` pin は **gate script の呼び出しに限定した、
既に landing 済みのより狭い緩和策**である。本節はそれとは別に、gate 由来かどうかに
関わらず **あらゆる scratch worktree に適用される repo 全体の規約**。

## unit test は `tests.rs` に分離する

**新規に書く `#[cfg(test)]` unit test は、対象ファイルへの inline `mod tests { ... }`
ではなく、同名の `tests.rs` (同階層に `<parent>/tests.rs`、または `mod.rs` 構成なら
同ディレクトリの `tests.rs`) に分離すること。** private item へのアクセスは
`tests.rs` 冒頭の `use super::*;` で維持される。1 ファイルに複数の独立した
test group がある場合は `crates/raikiri-style/src/property/tests.rs` +
`property/tests/*.rs`、`crates/raikiri-dom/src/document/tests.rs` +
`document/tests/*.rs` の形 (hub file が `mod <group>_tests;` を並べ、各 group を
`tests/<group>_tests.rs` に置く) に倣う。

`assert_eq!(a, b, "msg")` の message 部分は assertion 失敗時のみ評価されるため、
成功する test では coverage が必ず未カバーになる。inline `mod tests` は対象ファイルと
同一ファイルなので、この行が本体コードの coverage 表示に混入する。`tests.rs` /
`*_tests.rs` / `*-tests.rs` は cargo-llvm-cov の default `--ignore-filename-regex`
(`scripts/lib/patch_coverage.py` の `LLVM_COV_IGNORED_FILENAME_RE` 参照) に一致し、
無設定で coverage report から除外される。rustc 自身も tidy の `unit_tests` check で
同種の分離を強制している。

既存の inline test を全ファイル一括で移行する義務はない。触ったファイルの
inline test は分離する、新規ファイルは最初から `tests.rs` で書く、という段階移行。
一括移行は並行 worktree の diff 衝突を招くため意図的に避けている。

### orphan tests.rs の検出

`tests.rs` は **親ファイルに `mod tests;` 宣言が無いと、rustc が warning も error も
出さずに黙って一切コンパイルしない** (module tree 外のファイルは存在しないのと
同じ扱い)。分離の際は `scripts/orphan-tests-lint.sh` (gate §8.1(b) と CI の両方に
組み込み済み) で宣言漏れが無いことを確認すること。検出ロジックは
`scripts/lib/orphan_tests_check.py` 参照。

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:970c3bf2 -->
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
   bd dolt push
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
