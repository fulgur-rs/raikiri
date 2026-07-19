# Sprint 5 Parallel-Drain Cross-Scope Rollup (2026-07-19)

**性格**: 本 doc は per-scope retro (skill `raikiri-workflow:retro`) の scope 外にあり、user 明示要請の ad-hoc portfolio-level 集約。**規律変更提案は行わない** (per-scope retro が既に 6 件 bd decision 起票済み、human approver 待ち)。目的は 2 scope × 同日 drain という workspace 初事象を、per-scope retro が拾えない angle から可視化する。

## Aggregation of source retros

| Retro | Scope | Sprint | Tasks | Merges | Autonomous | Blocked/human filings | Advisor calls | Codex iters |
|---|---|---|---|---|---|---|---|---|
| [`2026-07-19-style-2.md`](./2026-07-19-style-2.md) | style | `sprint/style/2` | 2 (jzv + mvu) | ee8e986, 9bf2a4e | 2/2 (100%) | 0 | 0 | 3 (jzv hang 1 + jzv PASS 1 + mvu PASS 1) |
| [`2026-07-19-coord-3.md`](./2026-07-19-coord-3.md) | coord | `sprint/coord/3` | 3 (cif + 7i3 + yxq) | 151d3fc, de998d3, 044e6d5 | 3/3 (100%) | 0 | 1 (7i3 convention-conflict) | 5 (cif 1 + 7i3 2 + yxq 2 incl crash rerun) |
| **Aggregate** | (parallel) | Sprint 5 | **5** | 5 | **5/5 (100%)** | **0** | **1** | **8** productive + 2 abnormal (yxq 46 min silent-death + jzv 1h 15m hang) |

**Wall-clock**: sprint 5 は 2026-07-19 に session-style + session-coord 並行 dispatch、単日で全 drain 完了。session-style は jzv codex hang で 1h 15m 拘束されたが session-coord は independent に yxq crash + rerun を切り抜けて先行 drain。

## Cross-scope observations that only aggregate view sees

### Observation A: 1-day 5-task parallel drain — workspace-initial event

n=1 だが、以下 5 invariant を同時に達成した portfolio milestone:

1. 100% autonomous merge (5/5)
2. 0 `blocked/human` 新規 filing (sprint 5 期間中に加算なし)
3. 0 post-merge rework (5/5 merged commit の 24h 以内 fix 無し)
4. 0 cross-scope handoff activation (bd raikiri-spike-e51 Part B 非発火 — 2 scope 独立完結、94e のような cross-scope epic 依存無し)
5. 0 wall/cleanroom violation (style は raikiri-style leaf 維持、coord は raikiri-traits internal のみ)

これは coord/2 の 94e (1 P1 atomic wall-crossing) や style/1 の rbo+s85 (2 P2/P3 spec-heavy feature-add) と質的に異なる — **routine follow-up drain の並列化** が実運用で成立した状態。Portfolio-level 判断材料として:

- **Session concurrency n=2 が実運用 baseline に到達**: 「n=2 in-flight は例外」から「n=2 in-flight は routine」への状態遷移。
- **次 capacity 目標 n=3 in-flight** の検討開始が妥当 — 3 scope 同時 (style + coord + branch-*、または style + coord × 2 session-coord) の file-orthogonality + rule 支援 (§8.1.3 + §8.3.1) が n=3 で崩れないかを次サイクルで検証。

### Observation B: Rule ratification cycle now sub-sprint

これまで暗黙に「retro → 規律案 → human approve → rule file 編集 → 次 sprint 以降 field 検証」と 2+ sprint cycle 想定していた。実際には:

| Rule | Retro filed | Applied to raikiri-workflow | First field exercise | Result |
|---|---|---|---|---|
| `§8.1.3` (from `9ce`) | coord/2 (2026-07-19 早朝) | c813e14 (2026-07-19) | coord/3 × 3 merges (同日午後) | ✓ clean × 3 |
| `§8.3.1` (from `5fw`) | style/1 (2026-07-19 早朝) | c813e14 (2026-07-19) | style/2 jzv rebase × 2 (同日) + coord/3 non-fire (verified) | ✓ clean × 2 |
| `9y9 bundle` (spec-citation) | style/1 (2026-07-19 早朝) | c813e14 (2026-07-19) | mvu spec-review discipline (同日) | ✓ discipline validated + surfaced propose-side gap (`4er`) |

**サイクル時間**: 実質 1 日以下。retro → 提案 → 承認 → 実装 → 検証が同日で回転している状態。

Portfolio 意味:

- Rule 提案の実効性 (rule 提案が単に飾りでなく、実際に施行されて実効果を出しているか) が 1 sprint で検証可能な体制になった。
- 逆に、rule 追加が accumulate すると "meta-rule bloat" を招く可能性 — 今後 rule 数 monitoring が必要 (現在: `.claude/rules/*.md` 5 file、rule 追加候補 = 6 open decisions が仮に全 accept された場合 rule 数拡大)。
- **Recommend**: `raikiri-workflow` 側で rule 定期 audit を PMO 判断入力に追加 (次 retro が触れる)。

### Observation C: 6 open rule改善 decisions form a coherent 3-cluster shape

Sprint 5 が filing した 6 open bd decisions は無関係 6 件ではなく、明確な 3-cluster:

**Cluster 1: Codex reliability = 3-legged stool** (2 scope 由来、highest priority)

| bd id | 由来 | Coverage | Priority |
|---|---|---|---|
| `raikiri-spike-ton` | coord/3 (yxq 46-min silent-death) | Prompt hygiene: sandbox-aware prompt で cargo re-run 誘発を防ぐ | P2 |
| `raikiri-spike-anf` | coord/3 (yxq 46-min silent-death) | Detection: mtime staleness > 3 min → PID check で silent worker exit を検知 | P3 |
| `raikiri-spike-1ei` | style/2 (jzv 1h 15m hang) | Response playbook: 検知後の `/codex:cancel` + rebase + re-dispatch --fresh | P2 |

n=2 material hangs (yxq 46 min + jzv 1h 15m) が異なる scope で同日発生 → 独立 3 sprint の evidence が 1 日で集まった稀な状況。**human approver への推奨**: 3 件 bundle で `gate.md §8.3` の subsection layout として 1 PR で扱う (§8.3.2 prompt / §8.3.3 detection / §8.3.4 response、または単一 §8.3.2 として 3 clause 併記)。個別 accept でも良いが 3-legged 独立性が失われる。

**Cluster 2: 9y9 outward extensions** (2 scope 由来、healthy expansion pattern)

| bd id | 由来 | Coverage direction | Priority |
|---|---|---|---|
| `raikiri-spike-37n` | coord/3 (7i3 convention conflict) | Task-authoring 側 (planner): 既存 sibling pattern verification | P3 |
| `raikiri-spike-4er` | style/2 (mvu anchor fabrication) | Reviewer-spec 側 (propose): 提案する anchor の WebFetch 事前検証 | P3 |

`9y9` (spec-citation discipline) 本体は 3 clause (a: task-authoring primary-source verify / b: anchor-fragment > numeric section / c: reviewer-spec verifies own-task claims)。sprint 5 は (a) の "convention flavor" と (c) の "propose-side flavor" を独立に surface。

**Recommend to human approver**: 2 件を独立 acceptで扱っても bundle しても良いが、9y9 rule doc の見出し構造に沿って「9y9-2026-07-19 extensions bundle」として 1 PR で cross-ref。

**Cluster 3: New axis — coordinator dispatch authorization** (single scope 由来、独立)

| bd id | 由来 | Coverage | Priority |
|---|---|---|---|
| `raikiri-spike-2ea` | style/2 (3 amend security warnings) | Coordinator が single-purpose task worktree branch で amend を明示 authorize する場合の false-positive 抑制 | P3 |

style/2 単体由来、coord/3 は amend パターン非発火 (worktree の commit 手順が異なる)。**Recommend**: single-scope 発火なので優先度は低い、ただし style scope が続く限り再発必至。cluster 1/2 と bundle 不可。

### Observation D: Surfaced-by-discipline failure mode taxonomy 成熟

per-scope retro (style-2 S1/S2, coord-3 S1/S2/S5) は共通して **"hard checks が働いているから soft failure が可視化された"** 構造を示した。Portfolio 観点で:

- 5 sprint 累積で hard failure taxonomy (fmt drift / clippy break / test fail / merge conflict / wall violation / spec divergence) は routine automation で押さえ込まれている。
- Sprint 5 で surface した failure は全て soft:
  - Codex reliability (long-tail latency、silent worker death)
  - Prescriptive error in task authoring (convention + spec 両 flavor)
  - Amend security-warning false-positive
  - Cross-lens dedup on same-day follow-up (spec + debt 独立 filing、coord/3 S5)
- これらは hard checks が失敗するまで潜在化する mode、Sprint 5 は **hard checks が失敗しなかったからこそ soft mode の taxonomy が可視化された** cycle。

**Portfolio 判断**: 今後 sprint も同傾向 (hard invariant 保持 + soft mode 逐次 surface) を期待するのが realistic。Rule 追加の accumulate は soft mode 覆盖のために continue するが、**ratification cycle 1 日以下** (Observation B) が支えている限り "rule bloat" は成長速度に見合う品質改善に翻訳されている。

### Observation E: Cross-scope handoff (e51 Part B) 非発火の意味

sprint 4 (coord/2 = 94e) は cross-scope handoff の代表事象 (style-scope 3ps parent が coord-scope 94e phase B に依存)。Sprint 5 は 2 scope 独立で drain 完了、handoff 非発火。

**Positive interpretation**: 各 scope が独立 productive backlog を持てる状態 (post-94e leaf shape) が確立、無理な cross-scope 分割は不要になった。

**Concern**: これが持続すると M4 実装 kickoff 時 (M3 完了 → style scope + dom scope + traits scope 三つ巴 → 再び cross-scope handoff 集中) の準備が薄い可能性。**Recommend**: M4 kickoff 手前 sprint (次 or 次々) で cross-scope handoff protocol の warm-up dry-run を planner 判断で入れる。

### Observation F: Ready-queue 状態遷移 (9 → 17)

sprint 5 kickoff → drain の間に ready queue が 9 → 17 (+8) に増加。内訳:

- +5: sprint 5 drain 由来 followup children (ctk / rm8 / d6j / z3v / 2ng)
- +6: sprint 5 retro 由来 rule 改善 decisions (1ei / ton / 2ea / 37n / 4er / anf)
- +1: 3ni dup close ⇒ -1 (実質)
- 他 sprint 5 前から open だった backlog: 変化なし

**Portfolio 意味**: sprint 5 の付加産物 (5 followup child + 6 rule decision) は次 sprint の planner 候補に 11 pool を加えた。planner は次 sprint で「followup drain vs rule decision human approve 待ち vs M4 pre-work 継続」の 3 択判断が必要。

**Recommend**: 次 sprint planner は clusters 1/2/3 の rule decisions を human approve 経由で先に消化 (implementation cost 低い — rules/*.md 編集のみ) してから followup children drain に入る sequence を検討。rule 未 approve 状態が次 sprint drain 中に 3-legged stool 発火した場合、対応が ad-hoc になる可能性。

## Recommendations for human approver

**Priority 1**: Cluster 1 (Codex reliability 3-legged stool) を 1 PR で `gate.md §8.3` subsection に集約 accept。3 件独立 accept より coherent shape が保たれる。

**Priority 2**: Cluster 2 (9y9 extensions 2 件) を 9y9 rule doc に in-place amendment。planner-side (37n) + reviewer-spec-side (4er) の 2 clause 追加として 1 PR。

**Priority 3**: Cluster 3 (2ea amend authorization) を独立 accept、`autonomy.md` に「原則 4 candidate」または `gate.md §8.2` note として。

**Priority 4** (optional): rule 定期 audit を PMO 判断入力に追加する仕組み (rule 数 growth monitoring + effectiveness field exercise counting)。次 retro で触れる。

## Recommendations for next planner

**Sprint 6 kickoff の readiness check**:

1. Cluster 1/2/3 human approve 状態を確認
2. 未 approve があれば approve session を先に (planner が別 session で propose PR、user approve → merge into raikiri-workflow)
3. Approve 済み rule は `.claude/rules/*.md` に反映済 ⇒ sprint 6 で first field exercise
4. その後、followup children (ctk / rm8 / d6j / z3v / 2ng / 3ni-close) + M4/M5 pre-work + wpt lint bundle から次 ready-set 選定

**Session concurrency 判断**:

- Sprint 5 で n=2 実運用 baseline 到達。
- Sprint 6 で n=2 継続、または n=3 in-flight を検討開始。
- n=3 検討時は file-orthogonality の事前検証 (§8.1.3 が 3 session 交互 landing で崩れないか) が焦点。

**M3 → M4 kickoff 準備**:

- M3 epic は open、M4 pre-work は sprint 5 で jzv/mvu/cif/7i3 の 4 task 消化。
- M4 real 実装 kickoff は M3 完了待ち — 現時点で next sprint 対象外。
- M4 kickoff 手前 sprint で cross-scope handoff dry-run を planner 判断で入れることを推奨 (Observation E).

## Recommendations for PMO issue

`raikiri-spike-pmo` に本 doc への reference を append (post 済 style-2 + coord-3 の individual PMO note に加えて、workspace-wide rollup として本 doc link)。人間 approver が個別 retro を読まずに portfolio-level 判断できるように。

## Sprint 5 aggregate — retros filed 6 decisions:

- `raikiri-spike-1ei` (P2, style/2) — Codex hang response playbook
- `raikiri-spike-ton` (P2, coord/3) — Sandbox-aware Codex prompt discipline
- `raikiri-spike-2ea` (P3, style/2) — Amend authorization on task worktree
- `raikiri-spike-37n` (P3, coord/3) — Task-authoring convention-consistency
- `raikiri-spike-4er` (P3, style/2) — Reviewer-spec anchor propose-side verification
- `raikiri-spike-anf` (P3, coord/3) — Codex worker liveness heuristic

いずれも status=open (proposed)、human approve 待ち。

---

**本 doc の位置付け**: `raikiri-workflow:retro` skill 適用外の ad-hoc rollup。skill 完遂は per-scope retro (`2026-07-19-style-2.md`, `2026-07-19-coord-3.md`) 側で既に完了。本 doc は追加提案・追加 bd decision を起票せず、既存 6 decisions を portfolio 目線で cluster 化して human approve 順序を推薦する observation 集約。

**Next artifacts**: sprint 5 系 3 doc (style-2 / coord-3 / 本 rollup) は user 判断で 1 commit `docs(retros): sprint 5 parallel-drain (style-2 + coord-3 + cross-scope rollup)` で bundle 化推薦。
