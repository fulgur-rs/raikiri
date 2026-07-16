# raikiri-spike-bj8: find_by_tag を iterative DFS に統一

> **Status:** COMPLETED

**Goal:** `crates/raikiri/tests/build_cascaded.rs` の `find_by_tag` helper を recursive DFS から iterative DFS に書き換え、production `raikiri-style::ruletree::walk_and_collect` (roborev job 199) と同じ pattern に揃える。

**Architecture:** `Vec<NodeId>` stack で explicit iterative 実装。pre-order (sibling 間 document order) を保つため children を reverse push。

**Tech Stack:** Rust (MSRV 1.89+、let-chains stable in project edition)

## Acceptance Criteria

- [x] find_by_tag が recursion を含まない (inner fn walk 削除、Vec stack で iterative)
- [x] cargo test -p raikiri --test build_cascaded が全 pass (behavior-preserving)
- [x] Doc comment に walk_and_collect / job 199 への参照が含まれる
- [x] Signature が不変で、呼び出し側 15 箇所に変更なし

## Implementation Summary

**File changed:**
- `crates/raikiri/tests/build_cascaded.rs` (lines 19-37 → 19-42): recursive fn walk(inner) を iterative while loop に置換

**Verification:**
- 9/9 tests passing (behavior-preserving)
- cargo check clean
- cargo clippy clean (no needless_collect warning due to lifetime constraints match with production pattern)

**Pattern fidelity:** `Vec<NodeId>` stack の reverse-push による pre-order traversal が `walk_and_collect` と同じ構造。
