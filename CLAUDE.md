# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## Source comment language

Write code comments in English, including `//`, `///`, `//!`, block comments,
CSS comments, and comments in scripts and manifests. Translate explanations,
not program text. Keep exact Unicode glyphs in examples, test fixtures, and
quoted specification text when their spelling is necessary to explain behavior.
Do not translate string literals or user-facing content under this rule.

## docs/superpowers/ は flow 情報

`docs/superpowers/` 配下 (plans / retros / specs / sprints) は **flow 情報であって stock
情報ではない**ため、**tracked にしない**。新規 doc も untracked のままにし、`git add -f`
で追加しないこと。規則の正典は **AGENTS.md の「docs/superpowers/ は flow 情報
(tracked にしない)」節**を参照。

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

## Conventions & Patterns

Topic-scoped guides under `.claude/rules/` (read on demand, not loaded into context by default):

- `.claude/rules/no-internal-jargon-in-source-comments.md` — checked-in source comments under `crates/**` (Rust doc comments, UA CSS, etc.) must not reference bd issue IDs, milestone/epic labels, internal audit/workflow process names, or agent-memory paths; keep spec citations and technical rationale intact.
