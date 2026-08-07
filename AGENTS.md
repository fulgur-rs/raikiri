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

## `crate::…` pointer は intra-doc link で書く

**doc comment (`///` / `//!`) 中の `crate::…` 参照は intra-doc link
(``[`crate::foo::Bar`]``) で書く。plain code span (``` `crate::foo::Bar` ```) で書かない。**
module-relative な pointer (``` `cascade::foo` ```) も同じ — audit grep を `crate::` だけで
書くとこの形を取りこぼす。

理由は実証済み: plain code span の pointer は **誤っていても永久に検出されない**。
bd raikiri-spike-nqkj では lens が提案した ``` `crate::computed::ComputedBorder` ```
が実際には誤った path だったが、intra-doc link へ変えて初めて
`RUSTDOCFLAGS="-D warnings" cargo doc` が unresolved link として落とした
(bd raikiri-spike-ulzv)。

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

   **hatch の閾値 (bd raikiri-spike-uy4g で確定)**: 「widening が gate で enforce
   されるなら可、補助 command (`--document-private-items`) でしか enforce されないなら
   本項の `mod@` 代替に倒す」。判定 command は上の「わざと壊して確かめる」手順そのまま
   (対象 item を 1 つだけ private に戻し、gate command で red になるかを見る) —
   追加の道具は要らない。理由は bd raikiri-spike-8yj6 の PMO decision
   (2026-08-07): gate §8.1 の doc build には `--document-private-items` を足さないため、
   補助 command でしか検証されない widening は **検証利得ゼロで visibility 拡大だけが
   残る** ことが確定した。

   **適用結果 (bd raikiri-spike-ulzv が昇格した 6 件、rustc 1.89.0 で実測)**:
   - **gate-enforced (pub(crate) を維持)**: `crates/raikiri-style/src/cascade.rs` の
     `apply_value` / `resolve_relative_weight` / `resolve_inheritance` — 対象を private
     に戻すと gate (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`) が
     unresolved link で red になる (gate が実際にこの doc link を検証している)。
   - **mod@ へ降格 (private に戻した)**: `crates/raikiri-style/src/page.rs` の
     `absolutize_in_page_context` / `PageSpecificity`、`crates/raikiri-style/src/rule.rs`
     の `DeclParser` — 対象を private に戻しても gate は green のまま
     (補助 command でしか red にならない)。referrer 側の doc link は次の形に書き換えた:
     - `page.rs` の 2 件は module/item の同名衝突が無いため、既存の
       ``[`crate::page::PageSpecificity`]`` 形の full path link で gate 上は足り、
       書き換え不要 (`crates/raikiri-style/src/cascade.rs` / `rule.rs` の referrer 3 箇所)。
       ただし補助 command では引き続き unresolved (private item への full path link は
       `--document-private-items` 下でも解決しない) — これは想定内で、gate が
       enforce しない以上ここに追加コストは掛けない。
     - `rule.rs` の `DeclParser` は trait full path (規約 5) と組み合わさっており
       naive な demotion では文が崩れるため、``[`mod@crate::rule`] の `DeclParser` ``
       + trait 側 link の形に書き換えた (`crates/raikiri-style/src/property.rs`,
       `crates/raikiri-style/src/counter_style.rs`)。この形は補助 command でも
       clean に解決する (module 自身が link 元から見えるため)。
4. **同じ scope に同名の item がある module は `mod@` を付ける。** `raikiri-style` では
   `pub mod cascade` と `pub use cascade::{…, cascade}` が module と関数を同名で crate root
   に置くため ``[`mod@crate::cascade`]`` と書く。無印は ambiguous link になる。
5. **trait impl の method は trait 側の full path で書く** —
   ``[`cssparser::DeclarationParser::parse_value`]``。trait を宣言している crate からの path
   なら import 無しでも依存 crate のものでも解決する (rustc 1.89.0 で実測)。書けない場合に限り `Type` までを
   link 化し method 名を code span で添える。

### ⚠️ 「link 化した = 検証された」ではない

gate `§8.1` の `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` が検証するのは
**rustdoc が document する doc に書かれた link だけ**である。**どの位置が検証されるかは
source の見た目から予測できない。**

これは推測ではない。bd raikiri-spike-ulzv の gate §8.2 で **4 iter にわたり分類を書いては
実測で反証される**ことを繰り返した。反転させる変数として少なくとも **module の nest、
`pub use` re-export の有無、`#[doc(hidden)]`、`#[test]` 属性、target 自身の可視性**の
5 つが確認されており、**網羅は取れていない**。

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

gate 側の doc command を広げるかの判断と、**上記 5 変数の実測 table** は
bd raikiri-spike-8yj6 が持つ。

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

- **plain `//` comment には rustdoc が届かない。** link 化しても検証されない
  (bracket を書くと「検証済みに見えるのに実は未検証」でかえって危険)。→ bd raikiri-spike-vyse
- **既存の `#[test]` item doc に残る短縮 link** (``[`expand_shorthand_into`]`` 等) は
  opt-out 3 の位置なので未検証。既存分の一括変換は bd raikiri-spike-acsw。
  **新規に書く doc では opt-out 3 に従い code span にすること。**
- **本規約には enforcement が無い。** 規約に従わない新規記述は何も止めない
  (実測: 規約 landing 前の 3 merge が bare pointer を 7 site 追加した)。→ bd raikiri-spike-luxp
- **toolchain 依存がある。** intra-doc link の解決は rustc version で変わる。
  `rust-toolchain.toml` の pin (1.89.0) では出ない unresolved link が新しい toolchain では
  出る実例があるため、toolchain bump 時は本節の command を再走させること。
- **gate §8.1 の doc build は private / pub(crate) item の doc link を検証しない。**
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` (gate §8.1 が走らせる
  既定 command) は `--document-private-items` を付けないため、public item の doc comment
  内の link しか解決しない。`crates/raikiri-style/src/*.rs` の bracket link 114 occurrence
  実測 (bd raikiri-spike-ulzv、commit 889fad5 時点) では、うち 47 occurrence
  (pub(crate) 25 + private 22) が既定 gate command では未検証。
  **PMO decision (2026-08-07、bd raikiri-spike-8yj6)**: `--document-private-items` は
  gate に **足さない**。根拠は doc build 時間の増加と missing_docs 相当の露出面拡大
  (raikiri-style 以外の crate は未確認) というコストが、47 occurrence の未検証リスクに
  見合わないという判断。**既知の未修正 dangling として以下 3 件が raikiri-dom 側に残っている**
  (本 decision では修正しない、gate red 化の前提条件にはならないが実 dangling である):
  - `crates/raikiri-dom/src/running.rs:529` — `[MarginBoxFragment]` (no item named
    MarginBoxFragment in scope)
  - `crates/raikiri-dom/src/fonts.rs:616` — `` [`FontWarn::ReadRejected*`] `` (enum
    `FontWarn` に該当 variant 無し。末尾 `*` は glob のつもりだが intra-doc link に
    glob 記法は無い)
  - `crates/raikiri-dom/src/layout.rs:96` — `` [`raikiri_style::BoxSizing`] `` (no item
    named BoxSizing in module raikiri_style)
  詳細な実測 (`--cfg test` option の E0432 blocker、5 つの反転変数、blind zone の全体像)
  は bd raikiri-spike-8yj6 の comment 履歴を参照。

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
