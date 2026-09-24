# ソースコードコメントに beads ID / 内部ジャーゴンを書かない

## Rule

`crates/**` 配下の tracked なソースファイル (Rust doc comment、UA CSS コメント等) には、
以下を **書かない**:

- beads issue ID (`bd raikiri-spike-XXXX` のような tracker 参照)
- milestone/epic 番号 (`M1.4a`, `M2+`, `Epic 4` など raikiri 内部のロードマップ表記)
- 内部 audit/workflow プロセス名 (`milestone-gap-audit item`, `reviewer:spec pass`,
  `audit Category M9` など raikiri-workflow の運用固有語)
- agent memory への参照 (`memory raikiri-implementation-independence` など — これは
  Claude Code の個人 memory store 内のファイルを指しており、bd はおろか
  **別の agent session からも参照不可能**)
- 一過性の検証メモ (`verbatim confirmed 2026-08-14 WebFetch`, `re-verified 2026-08-15`
  のような「誰かがいつ確認したか」を刻んだ日付つき記録)

## Why

- これらは全て **bd やこのセッションの会話文脈にアクセスできる読み手にしか意味がない**。
  ソースコメントは spec 由来の rationale を説明するためのものであり、想定読者は
  将来の contributor・OSS reader・そして raikiri の設計目的である blitz backport の
  レビュアーまで含む。誰も bd DB を持っていない。
- bd issue は close・rename・`bd compact` による要約の対象になる。ソース中の ID 参照は
  将来ほぼ確実に dangling reference になる。
- memory への参照は特に有害。memory は **そのセッションを実行している agent 個体の
  ローカル store** であり、リポジトリの一部ではない。別 agent・別マシン・人間の
  contributor から見ると、文字通り存在しないファイルを指すことになる。

## How to Apply

書こうとしているコメントについて、「bd を開いたことがなく、このセッションの会話も
見ていない読み手にとって意味が通るか」を自問する。通らなければ、tracker 参照を落として
**技術的な理由だけ** を残す。

- **代わりに書くもの**: spec section 番号・URL、関数名や file:line への直接参照、
  実装の制約自体の平叙文説明 (例:「property.rs の parse_value に background
  shorthand の parser arm が無いため longhand で代替」)。これらは bd が無くても
  自己完結する。
- **bd に書くもの**: 「いつ・誰が・どう確認したか」「milestone のどこに位置するか」
  「audit のどの項目か」といった運用文脈は bd issue の description/comment 側の仕事。
  ソースへ転記しない。
- **適用範囲**: `crates/**` 配下の tracked ファイル全般 (Rust doc comment、UA CSS
  コメント等)。`AGENTS.md` / `CLAUDE.md` / `.claude/rules/**` / bd issue 本文自体は
  対象外 (これらは元から agent 向け内部文書であり、bd 参照が本来の役割)。
  `docs/superpowers/**` は untracked な flow 情報のため、そもそも本ルールの対象外。

## Before / After

`crates/raikiri-html/src/ua/minimal.css` より抜粋:

Before:

```css
/* dialog (the 7th §flow-content-3 residue element, bd raikiri-spike-cfbo)
 * is deliberately NOT added here yet. ...
 * Deferred as a whole unit to bd raikiri-spike-wezw, a followup
 * blocked on bd raikiri-spike-kxki (raikiri-dom empty-value-attribute
 * bug), rather than landed half-correct. */
```

After:

```css
/* dialog (the 7th §flow-content-3 element not covered above) is
 * deliberately NOT added here yet. ...
 * Deferred as a whole unit, blocked on a raikiri-dom
 * empty-value-attribute bug, rather than landed half-correct. */
```

`:not()` 未対応、`attr()` の empty-value 正規化バグ、といった技術的な理由は完全に
残したまま、tracker ID だけを落としている。
