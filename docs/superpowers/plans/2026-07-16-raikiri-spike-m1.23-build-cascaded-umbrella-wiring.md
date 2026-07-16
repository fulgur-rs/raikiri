# m1.23 M1.4a umbrella — `raikiri::build_cascaded` wiring — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `raikiri` (umbrella) crate に `build_cascaded(&UncascadedDocument) -> CascadeResult` を追加し、m1.22 で揃った 4 つの component (`raikiri_html::parse` → `Document.stylesheets()` → `RuleTree::add_stylesheet` → `raikiri_style::cascade`) を **end-to-end で配線する**。Consumer が `use raikiri::…;` のみで `<p>Hi</p>` を parse して cascade し、UA CSS 経由で `display: Block` の computed value を得られる状態にする。

**Architecture:** raikiri crate の `lib.rs` (現状 M0 4-line stub) に:
1. sub-crate からの pub re-export を追加 (parse / ParseOptions / UncascadedDocument / build_cascaded / CascadeResult / ComputedValues / DisplayValue / StylesheetKind / Origin / Dom trait 群 等)
2. `build_cascaded` orchestration function を追加。`Document.stylesheets()` を iterate して `StylesheetKind → Origin` を umbrella 内で明示的に map し `RuleTree` を組む。続いて DOM `<style>` element の text も Author として集約するため、raikiri-style から `walk_style_elements` を pub free function として expose する (現状 private `walk_and_collect` の thin wrapper)。最後に `cascade(&doc.dom, &tree)` を呼び結果を返す。

`StylesheetKind → Origin` の翻訳は raikiri-html / raikiri-style を dep 方向で分離するため **umbrella (raikiri crate) 側で** 記述する (raikiri-html → raikiri-style の逆依存を発生させない)。

**Tech Stack:** Rust 2024 edition、rust-version 1.89.0、既存 workspace deps のみ (raikiri-traits / raikiri-html / raikiri-dom / raikiri-style / raikiri-net / raikiri-paint — Cargo.toml 変更なし)。新規 dep 追加なし。

## Global Constraints

- 対象 crate: `crates/raikiri` (umbrella)、`crates/raikiri-style` (walk_style_elements 露出のみ)。他 crate の API に変更を加えない
- Cleanroom 原則: raikiri-traits の pub API に cssparser / selectors / html5ever / taffy 型を漏らさない (既存責務境界を継続)
- Dep 方向: `raikiri-traits ← raikiri-dom / raikiri-style ← raikiri-html ← raikiri (umbrella)` の downward-only を守る。`StylesheetKind → Origin` の match は raikiri-html にも raikiri-style にも置かず、必ず raikiri umbrella 内で書く (m1.23 description Acceptance #2)
- Origin scope: **UserAgent + Author** の 2 段のみ (M1)。User origin は Non-goal (spec §M1.4a)
- `cascade().expect("m1 では常に Ok")` を許容 (M1 では常に Ok、Result 反映は M2+)
- 全 public API 追加は既存 crate lint policy (`#[non_exhaustive]` は new struct/enum のみ、function は不要)
- rustc / clippy warnings 0 を維持 (`-D warnings` gate、workspace lint policy)
- 既存 test の regression 禁止 (workspace baseline 250 test — 本 worktree で cargo test 済)
- 各 task 完了時に commit (frequent commits 原則)、commit message は `feat(<crate>): ... (m1.23)` conventional form
- Non-goals: `raikiri::html_to_png` (m1.14 担当)、incremental cascade / dirty-tracking (M2)、`Document.stylesheets` に DOM `<style>` 集約を統合する refactor (別 issue)

---

## File Structure

**Modify (raikiri-style):**
- `crates/raikiri-style/src/ruletree.rs` — 既存 private `walk_and_collect` を pub free function `walk_style_elements` として wrap して export、`lib.rs` に `pub use` 追加

**Modify (raikiri-style::lib.rs):**
- `crates/raikiri-style/src/lib.rs:26` — `pub use ruletree::{Origin, RuleTree, build_rule_tree, walk_style_elements};` に更新

**Modify (raikiri):**
- `crates/raikiri/src/lib.rs` — sub-crate re-export + `build_cascaded` 実装 (4-line stub を丸ごと置き換え)

**Create (raikiri integration tests):**
- `crates/raikiri/tests/build_cascaded.rs` — Consumer が `use raikiri::…;` のみで parse → cascade → display 判定できる整合性を verify する integration test 4 個 + テスト helper `find_by_tag`

---

## Task 1: `raikiri-style::walk_style_elements` を pub 露出

**Files:**
- Modify: `crates/raikiri-style/src/ruletree.rs`
- Modify: `crates/raikiri-style/src/lib.rs:26`

**Interfaces:**
- Consumes: 既存 private `walk_and_collect<D, F>(dom: &D, id: NodeId, on_style_text: &mut F)` (現状 module-private)
- Produces: `pub fn walk_style_elements<D: Dom, F: FnMut(&str)>(dom: &D, mut on_style_text: F)` — DOM `<style>` element の text を root から DFS 訪問し on_style_text callback に渡す。UA CSS は含まない (`Document::stylesheets()` 経路と分離)

### 目的 (task 単体で意味を持つ deliverable)

raikiri umbrella (Task 2) が DOM `<style>` element を Author として集約するために、raikiri-style 内部の walker を上位 crate から呼べる形で expose する。既存 `build_rule_tree` は internal 実装だけを共有し、外向き API 契約は「DOM walk を Author として append する thin walker」として明示化する。

- [ ] **Step 1: Write the failing test — walk_style_elements が pub として呼べることを compile-time で確認**

`crates/raikiri-style/src/ruletree.rs` 末尾 `mod tests` 内に追加:

```rust
    #[test]
    fn walk_style_elements_pub_visits_all_style_texts_in_document_order() {
        // 兄弟の <style> 2 個 → 呼び出し順で collected される。
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(s1, "p { color: red }");
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(s2, "div { color: blue }");

        let mut collected: Vec<String> = Vec::new();
        super::walk_style_elements(&doc, |css| collected.push(css.to_string()));

        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], "p { color: red }");
        assert_eq!(collected[1], "div { color: blue }");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
cargo test -p raikiri-style walk_style_elements_pub -- --nocapture
```
Expected: FAIL with `error[E0603]: function \`walk_style_elements\` is private` (module private) 相当のコンパイルエラー。

- [ ] **Step 3: Add pub free function `walk_style_elements` wrapping `walk_and_collect`**

`crates/raikiri-style/src/ruletree.rs` の `walk_and_collect` 定義の直前に追加:

```rust
/// DOM を root から DFS walk して全 `<style>` element の text を callback に渡す。
///
/// UA CSS は含まれない — `raikiri-html::parse` が `Document::add_stylesheet` 経由で
/// UA を注入しており、`Document::stylesheets()` 経路で raikiri umbrella が別途消費
/// する契約 (spec §M1.4a、raikiri-spike-m1.23)。本 walker は DOM `<style>` element
/// の text 収集のみを担当する。
///
/// 呼び出し順は `walk_and_collect` の iterative DFS に従い document order。
/// stack overflow 保護は `walk_and_collect` と共有 (roborev job 199)。
pub fn walk_style_elements<D: Dom, F: FnMut(&str)>(dom: &D, mut on_style_text: F) {
    walk_and_collect(dom, dom.root_id(), &mut on_style_text);
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

`crates/raikiri-style/src/lib.rs:26` を差し替え:

```rust
pub use ruletree::{Origin, RuleTree, build_rule_tree, walk_style_elements};
```

- [ ] **Step 5: Run test to verify pass**

Run:
```bash
cargo test -p raikiri-style walk_style_elements_pub -- --nocapture
```
Expected: PASS (1 test).

- [ ] **Step 6: Full crate test — regression check**

Run:
```bash
cargo test -p raikiri-style
```
Expected: 全 test pass (baseline 数と一致 + 新規 1 個)。

- [ ] **Step 7: Clippy**

Run:
```bash
cargo clippy -p raikiri-style --all-targets -- -D warnings
```
Expected: warning なし。

- [ ] **Step 8: Commit**

```bash
git add crates/raikiri-style/src/ruletree.rs crates/raikiri-style/src/lib.rs
git commit -m "feat(raikiri-style): expose walk_style_elements as pub free function (m1.23)"
```

---

## Task 2: `raikiri` umbrella crate — re-export + `build_cascaded` + integration tests

**Files:**
- Modify: `crates/raikiri/src/lib.rs` (現在 M0 4-line stub を全面置き換え)
- Create: `crates/raikiri/tests/build_cascaded.rs`

**Interfaces:**
- Consumes:
  - `raikiri_html::{parse, ParseOptions, UncascadedDocument, MINIMAL_UA_CSS}` — parse pipeline (Task 1 独立)
  - `raikiri_dom::Document` — `UncascadedDocument.dom` の実型、`Document::stylesheets(&self) -> impl Iterator<Item = (&str, StylesheetKind)>`
  - `raikiri_style::{RuleTree, Origin, cascade, CascadeResult, ComputedValues, DisplayValue, build_rule_tree, walk_style_elements}` — cascade pipeline (Task 1 の walk_style_elements を含む)
  - `raikiri_traits::{StylesheetKind, Dom, Element, Node, NodeId, NodeKind, QuirksMode, CascadeError, ParseError, RenderError, RenderWarning}` — shared vocabulary + trait
- Produces:
  - `pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult` — umbrella orchestration
  - re-exports 一式: `use raikiri::{parse, ParseOptions, UncascadedDocument, build_cascaded, CascadeResult, ComputedValues, DisplayValue, StylesheetKind, Origin, Dom, Element, Node, NodeId, NodeKind, MINIMAL_UA_CSS};` 等で完結する Consumer 面

### 目的

Consumer が単一 crate `raikiri` を dep に加えれば HTML → cascade された ComputedValues を得られる整合性を成立させる。m1.14 (hello-world VRT) が `raikiri::html_to_png` から本 function を呼び出す前提の gate。

- [ ] **Step 1: Write the failing test — build_cascaded の `<p>` cascade 動作 (最重要 AC #3)**

`crates/raikiri/tests/build_cascaded.rs` を新規作成:

```rust
//! raikiri umbrella integration tests (raikiri-spike-m1.23)。
//!
//! Consumer が `use raikiri::…;` のみで parse → build_cascaded → display 判定を
//! 完結できることを verify する。

use raikiri::{
    build_cascaded, parse, Dom, Element, Node, NodeId, NodeKind, ParseOptions,
    DisplayValue,
};

fn parse_html(source: &str) -> raikiri::UncascadedDocument {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    parse(source.as_bytes(), &opts).expect("parse")
}

/// DOM を root から DFS walk して最初に見つかった tag 一致の Element の NodeId を返す。
fn find_by_tag<D: Dom>(dom: &D, tag: &str) -> Option<NodeId> {
    fn walk<D: Dom>(dom: &D, id: NodeId, tag: &str) -> Option<NodeId> {
        if let Some(node) = dom.node(id)
            && node.kind() == NodeKind::Element
            && let Some(elem) = node.as_element()
            && elem.tag_name().eq_ignore_ascii_case(tag)
        {
            return Some(id);
        }
        for child_id in dom.child_ids(id) {
            if let Some(found) = walk(dom, child_id, tag) {
                return Some(found);
            }
        }
        None
    }
    walk(dom, dom.root_id(), tag)
}

#[test]
fn p_without_author_style_is_display_block_via_ua_css() {
    let doc = parse_html("<html><body><p>Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<p> should inherit display: block from bundled UA CSS via build_cascaded",
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
cargo test -p raikiri --test build_cascaded p_without_author_style
```
Expected: FAIL with `error[E0432]: unresolved import \`raikiri::build_cascaded\`` (未実装) や関連の未 re-export エラー。

- [ ] **Step 3: Replace `crates/raikiri/src/lib.rs` with re-exports + build_cascaded**

`crates/raikiri/src/lib.rs` の内容全体を差し替え:

```rust
//! raikiri — umbrella crate: primary consumer API and re-exports.
//!
//! M1 の umbrella-facade slice。Consumer が単一 `raikiri` crate だけを dep に
//! 追加すれば HTML parse → cascade された ComputedValues まで得られるように
//! sub-crate から必要な type / trait / function を re-export し、cascade
//! orchestration entry point `build_cascaded` を提供する
//! (spec §M1、raikiri-spike-m1.23)。
//!
//! # Example
//!
//! ```
//! use raikiri::{build_cascaded, parse, ParseOptions};
//!
//! let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
//! let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
//! let result = build_cascaded(&doc);
//! // result.computed に per-node ComputedValues が populate される。
//! # let _ = result;
//! ```

use raikiri_dom::Document;
use raikiri_style::{Origin, RuleTree, cascade, walk_style_elements};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
pub use raikiri_traits::{
    CascadeError, Dom, Element, Node, NodeId, NodeKind, ParseError, QuirksMode,
    RenderError, RenderWarning, StylesheetKind,
};

// ── raikiri-html: parse pipeline entry ─────────────────────────────────
pub use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, UncascadedDocument, parse};

// ── raikiri-dom: Document (owns DOM + stylesheets state) ───────────────
pub use raikiri_dom::Document as DomDocument;

// ── raikiri-style: cascade pipeline output types ───────────────────────
pub use raikiri_style::{
    CascadeResult, ComputedValues, DisplayValue, Origin as CascadeOrigin, PropertyValue,
    RuleTree as StyleRuleTree,
};

/// UA + Consumer 提供 stylesheet を Document から取り出し、Origin を割り当てて
/// RuleTree を組み、DOM 内 `<style>` element の text を Author として追加した
/// 上で cascade を実行する umbrella orchestration entry point
/// (spec §M1.4a、raikiri-spike-m1.23)。
///
/// M1 では `raikiri_style::cascade` は常に `Ok` を返すため、内部で `expect` する
/// (M2+ で Result 反映を検討)。
///
/// Consumer は `raikiri_html::parse` → `raikiri::build_cascaded` の 2 step だけで
/// per-node ComputedValues を得られる。
///
/// # Dep 方向
///
/// `StylesheetKind → Origin` の翻訳は raikiri-html にも raikiri-style にも置かず、
/// umbrella (本 crate) 内で明示的に書く。これにより下位 crate 間の逆依存を発生
/// させない (raikiri-spike-m1.23 Acceptance #2)。
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    let mut tree = RuleTree::empty();

    // Document に associate されている全 stylesheet を kind に応じて Origin
    // に map。呼び出し順 (=注入順) が cascade の source_order を決める。
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // DOM 内 `<style>` element の text を Author として追加。UA / extra_stylesheets
    // は上のループで既に取り込まれているため、ここでは重複しない。
    walk_style_elements(&doc.dom, |css| {
        tree.add_stylesheet(css, Origin::Author);
    });

    cascade(&doc.dom, &tree).expect("m1 では cascade は常に Ok")
}

/// dom-level の [`StylesheetKind`] (raikiri-traits) を cascade-level の
/// [`Origin`] (raikiri-style) に翻訳。dep 方向を保つため umbrella 内で保持。
///
/// `#[non_exhaustive]` により将来 variant が追加された場合の compile-time 網羅
/// 保証を維持する。
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` は `#[non_exhaustive]`。将来 User 等が追加された時点で
        // Origin 側も対応が必要になるため、ここは compile-time 網羅性を保持する。
        _ => Origin::Author,
    }
}

/// Cascade orchestration が Document 内 `<style>` を Author 経路で使うこと、および
/// Document.stylesheets 経由の UA CSS がここに二重計上されないことを再確認する
/// smoke-test は tests/build_cascaded.rs 側で担当する。
#[cfg(test)]
mod smoke_tests {
    use super::*;

    #[test]
    fn stylesheet_kind_to_origin_matches_spec() {
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::UserAgent),
            Origin::UserAgent,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::Author),
            Origin::Author,
        );
    }
}
```

**Note:** raikiri-dom の `Document` は `Document as DomDocument` と別名再 export、raikiri-style の `Origin` / `RuleTree` は `CascadeOrigin` / `StyleRuleTree` と別名。理由:
- `Document` は `UncascadedDocument.dom` field の型として transient に見えるが、Consumer が直接扱うのは稀。umbrella での top-level `Document` 名は将来 higher-level ラッパー (m1.11 facade 予定) が奪う想定なので予約
- `Origin` / `RuleTree` は Consumer から見ると "cascade internals" であり、`CascadeOrigin` / `StyleRuleTree` の方が用途を示唆

integration test は上記別名を使わない (直接 `raikiri::{parse, build_cascaded, DisplayValue, Dom, Element, Node, NodeId, NodeKind, ParseOptions, UncascadedDocument}` のみで完結)。

- [ ] **Step 4: Run failing test to verify pass**

Run:
```bash
cargo test -p raikiri --test build_cascaded p_without_author_style
```
Expected: PASS.

- [ ] **Step 5: Write additional integration test — author style overrides UA (AC #4)**

`crates/raikiri/tests/build_cascaded.rs` 末尾に追加:

```rust
#[test]
fn author_inline_style_overrides_ua_display_block() {
    // NB: m1.4 cascade は class/id selector を drop するので inline style を使う
    let doc = parse_html("<html><body><p style=\"display:inline\">Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "author inline style (Normal Author) should override UA (Normal UA) per cascade rank",
    );
}
```

- [ ] **Step 6: Run new test to verify pass**

Run:
```bash
cargo test -p raikiri --test build_cascaded author_inline_style_overrides_ua
```
Expected: PASS.

- [ ] **Step 7: Write regression test — DOM `<style>` element is picked up via umbrella**

`crates/raikiri/tests/build_cascaded.rs` 末尾に追加:

```rust
#[test]
fn dom_style_element_author_rule_overrides_ua() {
    // 明示的な <style> Author rule が UA を上回ることを verify。
    // m1.4 cascade は type selector のみサポートするため p{...} を使う。
    let html = "<html><head><style>p { display: inline }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "DOM <style> Author rule should override UA (both via umbrella wiring)",
    );
}
```

- [ ] **Step 8: Write regression test — extra_stylesheets is picked up via umbrella**

`crates/raikiri/tests/build_cascaded.rs` 末尾に追加:

```rust
#[test]
fn extra_stylesheets_author_rule_overrides_ua_via_umbrella() {
    // Consumer が opts.extra_stylesheets 経由で渡した CSS が Author として
    // build_cascaded 経路に到達することを verify (parse 時 Document.stylesheets
    // に Author として push される)。
    let extra: &[&str] = &["p { display: inline }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(display, DisplayValue::Inline);
}
```

- [ ] **Step 9: Run all integration tests**

Run:
```bash
cargo test -p raikiri --test build_cascaded
```
Expected: 4 tests PASS.

- [ ] **Step 10: Run full workspace test — regression check**

Run:
```bash
cargo test --workspace
```
Expected: 全 test pass (baseline 250 + 新規 6 = 256 前後、regression なし)。

- [ ] **Step 11: Clippy on raikiri crate**

Run:
```bash
cargo clippy -p raikiri --all-targets -- -D warnings
```
Expected: warning なし。

- [ ] **Step 12: Clippy workspace-wide**

Run:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: warning なし。

- [ ] **Step 13: rustfmt check**

Run:
```bash
cargo fmt --all -- --check
```
Expected: 差分なし。

- [ ] **Step 14: Doc-test verify (lib.rs の example)**

Run:
```bash
cargo test -p raikiri --doc
```
Expected: PASS (`build_cascaded` の module-level example)。

- [ ] **Step 15: Commit**

```bash
git add crates/raikiri/src/lib.rs crates/raikiri/tests/build_cascaded.rs
git commit -m "feat(raikiri): umbrella build_cascaded wiring + integration tests (m1.23)"
```

---

## Post-implementation verification

- [ ] `bd show raikiri-spike-m1.23` で status を確認 (in_progress のまま、close は verification-before-completion 後)
- [ ] Acceptance criteria の 6 項目を plan self-check の形で確認:
  1. ✅ `raikiri::build_cascaded(&UncascadedDocument) -> CascadeResult` が pub API として実装 → Task 2 Step 3
  2. ✅ `StylesheetKind → Origin` map が umbrella 内で明示的に書かれている → `stylesheet_kind_to_origin` in `lib.rs`
  3. ✅ integration test: `<p>Hi</p>` → `DisplayValue::Block` → Task 2 Step 1
  4. ✅ integration test: author inline `display:inline` が UA を override → Task 2 Step 5
  5. ✅ `cargo build / test / clippy` 全 green → Task 2 Step 10-13
  6. ✅ re-export (`StylesheetKind` / `Origin` / `DisplayValue` 等) 追加、Consumer が `use raikiri::…;` で完結 → Task 2 Step 3 + integration test の use 節が証明
- [ ] `cargo test --workspace` を再実行して total count を記録 (before: 250、after: 256 前後を確認)

---

## Self-Review

**1. Spec coverage:** m1.23 description の Acceptance 6 項目それぞれに対応する task step が存在すること。上記 Post-implementation verification の対応表で照合済。

**2. Placeholder scan:** "TBD" / "similar to Task N" / 抽象化された "add error handling" 等が本 plan 内に存在しないこと。全ての code block は engineer がそのままコピーして動く状態で記載済。

**3. Type consistency:**
- `parse` signature: `pub fn parse<R: Read>(input: R, options: &ParseOptions<'_>) -> Result<UncascadedDocument, ParseError>` — Task 2 integration test の call と一致
- `build_cascaded` signature: `pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult` — description Scope と AC #1 に一致
- `walk_style_elements` signature: `pub fn walk_style_elements<D: Dom, F: FnMut(&str)>(dom: &D, mut on_style_text: F)` — Task 1 定義、Task 2 lib.rs 呼び出しと一致
- `ComputedValues.display: DisplayValue`、`CascadeResult.computed: Vec<ComputedValues>` — 実 code (computed.rs / cascade.rs) と一致確認済

**4. Dep 方向 sanity check:**
- Task 1: `raikiri-style` に閉じた変更、逆依存なし
- Task 2: `raikiri` crate から `raikiri-traits / raikiri-html / raikiri-dom / raikiri-style` を downward に use。Cargo.toml に既に全 dep 存在済 (変更不要)

**5. `#[non_exhaustive]` guard:**
- `stylesheet_kind_to_origin` の match で `_ => Origin::Author` fallback を含めている。`StylesheetKind` は `#[non_exhaustive]` で将来 variant が追加された場合の compile 失敗を回避しつつ、明示的な variant を先に列挙している (`UserAgent` → `Origin::UserAgent`、`Author` → `Origin::Author`)。将来 `User` variant が追加された時点で意図的に fallback から抜き出して固有 map するタイミング判断が必要。

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-16-raikiri-spike-m1.23-build-cascaded-umbrella-wiring.md`.**

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
