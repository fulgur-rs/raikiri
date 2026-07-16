# raikiri-spike-m1.11 umbrella-facade — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** raikiri (umbrella) crate に **assembled document type `HtmlDocument`** と **3 pub API (`parse_html` / `plan` stub / `render_streaming` stub)** を実装し、下流 m1.13 (determinism) / m1.14 (hello-world VRT) / m1.15 (external consumer compile test) を unblock する。

**Architecture:** m1.23 で追加済の `build_cascaded` を土台に、以下を積む:
1. `raikiri-traits` に `RenderError::Unimplemented` variant + `PageDefaults` struct + `PageBox::US_LETTER` const 追加、`PageBox` の doc-comment と `A4` 定数を **CSS pt → CSS px** に切り替え
2. `raikiri` crate に `HtmlDocument` struct を新設 (M1 実装 = `(UncascadedDocument, CascadeResult)` の私設 wrapper、M2+ で `pub use raikiri_dom::Document` に置換可能な surface)
3. `parse_html<R: Read>` orchestrator (raikiri_html::parse → build_cascaded → HtmlDocument assemble)
4. `plan` / `render_streaming` の signature stub (`Err(RenderError::Unimplemented { .. })` 返却)
5. m1.15 前哨の integration test (`external_consumer.rs`) + m1.14 前哨として全 `#[non_exhaustive]` 型が external crate から constructable なことを pin

`HtmlDocument` の naming は blitz の `HtmlDocument` と 1:1 対応 (M6 blitz-compat shim 対応)、shape は raikiri 独自 (blitz code の持ち込みなし、cleanroom 準拠)。

**Tech Stack:** Rust 2024 edition、既存 workspace deps のみ (raikiri-traits / raikiri-html / raikiri-dom / raikiri-style)。新規 dep 追加なし。

## Global Constraints

- 対象 crate: `crates/raikiri-traits`、`crates/raikiri` (2 crate)。他 crate の pub API に変更なし
- Cleanroom 原則: blitz-html / blitz-dom / blitz を参照する code / doc-string 一切なし (memory `raikiri-implementation-independence` 準拠)。`HtmlDocument` は naming のみ blitz と一致、shape は raikiri 独自 (`UncascadedDocument + CascadeResult`)
- Dep 方向: `raikiri-traits ← raikiri-dom / raikiri-style ← raikiri-html ← raikiri (umbrella)` の downward-only を維持
- `#[non_exhaustive]` 新規 struct/enum: `HtmlDocument`, `PageDefaults`。variant 追加 `RenderError::Unimplemented` は additive (既存 enum 既に `#[non_exhaustive]`)
- rustc / clippy warnings 0 を維持 (`-D warnings` gate、workspace lint policy)
- 既存 test の regression 禁止 (workspace baseline 260 tests — 本 worktree で cargo test 済)
- 各 task 完了時に commit (frequent commits 原則)、commit message は `feat(<crate>): ... (raikiri-spike-m1.11)` conventional form
- `raikiri_dom::Document` / `raikiri_style::CascadeResult` の直接 pub re-export は維持 (m1.23 Consumer 契約継承)
- `RenderError::Unimplemented` variant は M2+ pagination 完了時に削除予定 (breaking change として release notes 明記) — 本 plan の scope 外
- CSS px 単位切替: `PageBox` の `A4` 数値変更 (595 → 793.7008、842 → 1122.5197) に伴い m1.6/m1.7 側の hardcode assertion を追随修正 (regression fix)
- Non-goals (本 plan で扱わない): `html_to_png` (m1.14 担当)、`render_batch` (M2+)、`render_streaming_from_html` (M2+)、`raikiri_dom::Document::assemble` 本実装 (M2+)、`plan` / `render_streaming` の pagination logic (M2+)

---

## File Structure

**新規作成:**

- `crates/raikiri/src/html_document.rs` — `HtmlDocument` struct + accessor 3 method (dom/cascade/stylesheet_sources)
- `crates/raikiri/src/parse.rs` — `parse_html<R: Read>` 本体 (parse → build_cascaded → assemble)
- `crates/raikiri/src/stubs.rs` — `plan` / `render_streaming` stub (Unimplemented Err のみ、M2+ 削除 scope)
- `crates/raikiri/tests/external_consumer.rs` — m1.15 前哨 integration test (3 tests)

**修正:**

- `crates/raikiri-traits/src/error.rs` — `RenderError::Unimplemented` variant 追加、Display/Error::source に arm
- `crates/raikiri-traits/src/page.rs` — `PageBox` 単位切替 (pt→px)、`A4` 値更新、`US_LETTER` const 追加、`PageDefaults` + `PageDefaultsBuilder` 新設
- `crates/raikiri-traits/src/lib.rs` — 新設 `PageDefaults` / `PageDefaultsBuilder` を pub re-export に追加
- `crates/raikiri/src/lib.rs` — module 宣言 (`html_document`, `parse`, `stubs`) + 拡張 pub re-export (raikiri-traits の error/status/plan/config/page/traits/strategy/resolver/policy/layout/symbol 系)
- `crates/raikiri-dom/tests/*` (regression fix) — 既存 test で PageBox 数値 hardcode があれば新値に追随
- `crates/raikiri-paint/tests/*` or `src/lib.rs` の `#[cfg(test)]` (regression fix) — 同上

**変更なし:**

- `crates/raikiri/Cargo.toml` — dep 追加なし
- `crates/raikiri/src/lib.rs` の既存 `build_cascaded` 実装 (m1.23) — 保持、`parse_html` から内部呼び出し
- `crates/raikiri/tests/build_cascaded.rs` (m1.23 既存) — 保持

---

## Task 1: `RenderError::Unimplemented` variant 追加

**Files:**
- Modify: `crates/raikiri-traits/src/error.rs:19-56` (enum variant 追加), `:58-102` (Display / Error::source arm 追加)

**Interfaces:**
- Consumes: 既存 `RenderError` enum、`std::fmt::Display` trait
- Produces: `RenderError::Unimplemented { feature: &'static str, migration_hint: &'static str }` variant (下流 Task 6 の stub が使用)

- [ ] **Step 1: Write the failing test**

`crates/raikiri-traits/src/error.rs` の末尾 `#[cfg(test)] mod tests` (存在しなければ新設) に追記:

```rust
#[cfg(test)]
mod unimplemented_variant_tests {
    use super::*;

    #[test]
    fn unimplemented_display_includes_feature_and_hint() {
        let err = RenderError::Unimplemented {
            feature: "plan",
            migration_hint: "M2+ で pagination 実装後に populate",
        };
        let s = format!("{err}");
        assert!(s.contains("plan"), "display must include feature: got {s:?}");
        assert!(s.contains("M2+"), "display must include hint: got {s:?}");
        assert!(s.contains("not implemented"), "display must include 'not implemented': got {s:?}");
    }

    #[test]
    fn unimplemented_source_is_none() {
        use std::error::Error;
        let err = RenderError::Unimplemented {
            feature: "render_streaming",
            migration_hint: "hint",
        };
        assert!(err.source().is_none(), "Unimplemented has no inner cause");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p raikiri-traits unimplemented_variant_tests 2>&1 | tail -10`
Expected: FAIL with "no variant or associated item named `Unimplemented` found for enum `RenderError`"

- [ ] **Step 3: Add variant to `RenderError` enum**

`crates/raikiri-traits/src/error.rs` の `pub enum RenderError` に (Io variant の直前で挿入、alphabetical ではなく semantic 順で末尾):

```rust
    /// その他 `std::io::Error` 系。
    Io(std::io::Error),

    /// M1 stub 段階の API に対する call。M2+ で実装完了時にこの variant は
    /// **削除される** (breaking change として release notes に明記)。Consumer
    /// は M1 期間中のみ pattern match し、M2+ upgrade 時に arm 削除でよい。
    /// `feature` は呼ばれた stub API の識別 (`"plan"`, `"render_streaming"` 等)。
    Unimplemented {
        /// M1 stub の API 名。
        feature: &'static str,
        /// Consumer 向け migration hint。
        migration_hint: &'static str,
    },
```

**注**: variant を Io の後ろに追加した理由 = M2+ 削除時に既存 variant の順序変化を最小化 (Diff readability)。

- [ ] **Step 4: Add Display arm**

`crates/raikiri-traits/src/error.rs` の `impl std::fmt::Display for RenderError` の match の末尾 (`Self::Io(_)` の後) に追記:

```rust
            Self::Io(_) => write!(f, "I/O error"),
            Self::Unimplemented {
                feature,
                migration_hint,
            } => {
                write!(
                    f,
                    "{feature} is not implemented in M1 (hint: {migration_hint})"
                )
            }
```

- [ ] **Step 5: Add Error::source arm**

`crates/raikiri-traits/src/error.rs` の `impl std::error::Error for RenderError` の `fn source` の match:

現状:
```rust
            Self::LimitExceeded { .. }
            | Self::Configuration(_)
            | Self::TargetDidNotConverge { .. } => None,
```

を以下に置換:
```rust
            Self::LimitExceeded { .. }
            | Self::Configuration(_)
            | Self::TargetDidNotConverge { .. }
            | Self::Unimplemented { .. } => None,
```

- [ ] **Step 6: Run tests to verify pass**

Run: `cargo test -p raikiri-traits unimplemented_variant_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 2 passed`

- [ ] **Step 7: Regression check**

Run: `cargo test -p raikiri-traits 2>&1 | grep -E "^test result:"`
Expected: 全 test suite `ok`, 既存 test count + 2 の新 test pass

- [ ] **Step 8: Commit**

```bash
git add crates/raikiri-traits/src/error.rs
git commit -m "$(cat <<'EOF'
feat(raikiri-traits): RenderError に Unimplemented variant を追加 (raikiri-spike-m1.11)

M1 stub 段階の plan / render_streaming が返す構造化 error variant を新設。
既存 Configuration(String) の意味論汚染を避け、M2+ 実装完了時に variant 単位
で削除できる shape (release notes に breaking change 記載予定)。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 2: `PageBox` px baseline + `US_LETTER` const

**Files:**
- Modify: `crates/raikiri-traits/src/page.rs:37-62` (PageBox struct doc + A4 const 値、US_LETTER 追加)
- Test: 同 file の `#[cfg(test)] mod tests` (新設 or 追記)

**Interfaces:**
- Consumes: (なし、既存 struct 修正)
- Produces: `PageBox::A4 = 793.7008 × 1122.5197 px`, `PageBox::US_LETTER = 816.0 × 1056.0 px` const (Task 3 PageDefaults default、Task 8 integration test #11 が消費)

- [ ] **Step 1: Write failing tests**

`crates/raikiri-traits/src/page.rs` の末尾に:

```rust
#[cfg(test)]
mod pagebox_px_baseline_tests {
    use super::*;

    #[test]
    fn a4_dimensions_match_css_px_conversion() {
        // 210mm × 297mm を CSS px (1/96 in) 換算:
        //   width  = 210mm × 96/25.4 ≈ 793.7008
        //   height = 297mm × 96/25.4 ≈ 1122.5197
        assert!(
            (PageBox::A4.width - 793.7008).abs() < 0.001,
            "A4.width should be ~793.7008 px, got {}", PageBox::A4.width
        );
        assert!(
            (PageBox::A4.height - 1122.5197).abs() < 0.001,
            "A4.height should be ~1122.5197 px, got {}", PageBox::A4.height
        );
    }

    #[test]
    fn us_letter_dimensions_match_exact_integers() {
        // 8.5in × 11in @ 96 DPI = 816 × 1056 px exactly
        assert_eq!(PageBox::US_LETTER.width, 816.0);
        assert_eq!(PageBox::US_LETTER.height, 1056.0);
    }

    #[test]
    fn pagebox_default_is_a4() {
        assert_eq!(PageBox::default(), PageBox::A4);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p raikiri-traits pagebox_px_baseline_tests 2>&1 | tail -20`
Expected: FAIL — `a4_dimensions_match_css_px_conversion` は現状 A4=595×842 で assertion fail、`us_letter_dimensions_match_exact_integers` は `US_LETTER` 未定義 (compile error)

- [ ] **Step 3: Update `PageBox` doc-comment and const values**

`crates/raikiri-traits/src/page.rs:31-56` を以下に置換:

```rust
/// PageBox — @page rule 解決結果 (size, margins, margin box slots)。
/// M1.6 layout-single-page で width / height + `A4` / `US_LETTER` const を populate。
/// margins / margin_boxes は M4 で populate。
///
/// **単位 = CSS px** (1 CSS px = 1/96 in in print context per CSS Values L4 §5.2)。
/// pt / mm / in への換算は Consumer 責務。
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageBox {
    /// Page 幅 (CSS px)。
    pub width: f32,
    /// Page 高 (CSS px)。
    pub height: f32,
    // M4 で populate:
    //   pub margins: Margins,
    //   pub margin_boxes: [Option<MarginBox>; 16],
}

impl PageBox {
    /// A4 portrait: 210×297 mm = **793.70 × 1122.52 px** (@ 96 DPI anchor)。
    /// CSS Paged Media Level 3 §7 default size。
    pub const A4: PageBox = PageBox {
        width: 793.7008,   // 210mm × 96/25.4
        height: 1122.5197, // 297mm × 96/25.4
    };

    /// US Letter portrait: 8.5×11 in = **816 × 1056 px** ちょうど。
    pub const US_LETTER: PageBox = PageBox { width: 816.0, height: 1056.0 };

    /// Construct a `PageBox` = `A4`。`#[non_exhaustive]` の下でも安定した
    /// zero-arg constructor を残すため保持。
    pub fn new() -> Self {
        Self::default()
    }
}
```

`impl Default for PageBox` は変更なし (既存 `Self::A4` を返す、A4 の値が更新されたので transitive に更新される)。

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p raikiri-traits pagebox_px_baseline_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Check downstream regression (raikiri-dom / raikiri-paint)**

Run: `cargo test -p raikiri-dom -p raikiri-paint 2>&1 | grep -E "FAIL|^test result:" | head -30`

- 全 `ok` → Step 6 に進む
- 一部 FAIL → Step 5b (regression fix) を実施

- [ ] **Step 5b (conditional): Regression fix in raikiri-dom / raikiri-paint tests**

失敗した test を grep で特定:
```bash
cargo test -p raikiri-dom -p raikiri-paint 2>&1 | grep -B2 "assertion.*failed"
```

hardcode された `595` / `842` / 依存する期待値を新値 (`793.7008` / `1122.5197`) に置換。単位変更のみで logic 不変。**必要な差分だけ**を適用 (無関係な test を触らない)。

再実行:
```bash
cargo test -p raikiri-dom -p raikiri-paint 2>&1 | grep -E "^test result:"
```
Expected: 全 `ok`

- [ ] **Step 6: Full workspace regression check**

Run: `cargo test --workspace 2>&1 | grep -E "FAIL|^test result: (ok|FAIL)" | head -30`
Expected: 全 test suite `ok`

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri-traits/src/page.rs
# もし Step 5b で他 crate に変更があれば同時 add:
# git add crates/raikiri-dom crates/raikiri-paint
git commit -m "$(cat <<'EOF'
feat(raikiri-traits): PageBox を CSS px 単位に切替 + US_LETTER const 追加 (raikiri-spike-m1.11)

CSS spec の canonical 単位 (px) に合わせ、pt/mm/in 換算は Consumer 責務に移す。
A4 = 793.7008 × 1122.5197 px (210×297mm @ 96 DPI)、
US_LETTER = 816 × 1056 px (8.5×11in @ 96 DPI)。
m1.6/m1.7 側の hardcode assertion を追随修正 (logic 不変、値のみ更新)。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 3: `PageDefaults` + `PageDefaultsBuilder`

**Files:**
- Modify: `crates/raikiri-traits/src/page.rs` (末尾に追記)
- Modify: `crates/raikiri-traits/src/lib.rs` (pub re-export に追加)

**Interfaces:**
- Consumes: `PageBox` (Task 2)
- Produces: `PageDefaults { page_box: PageBox }`, `PageDefaultsBuilder` (Task 5 parse_html signature の第 2 引数、Task 6 stubs、Task 7 re-export、Task 8 integration test が消費)

- [ ] **Step 1: Write failing tests**

`crates/raikiri-traits/src/page.rs` の末尾に (Task 2 の tests mod の後):

```rust
#[cfg(test)]
mod pagedefaults_tests {
    use super::*;

    #[test]
    fn pagedefaults_default_uses_a4() {
        let d = PageDefaults::default();
        assert_eq!(d.page_box, PageBox::A4);
    }

    #[test]
    fn pagedefaults_new_is_default() {
        assert_eq!(
            PageDefaults::new().page_box,
            PageDefaults::default().page_box
        );
    }

    #[test]
    fn pagedefaults_builder_sets_page_box() {
        let d = PageDefaults::builder()
            .page_box(PageBox::US_LETTER)
            .build();
        assert_eq!(d.page_box, PageBox::US_LETTER);
    }

    #[test]
    fn pagedefaults_builder_default_matches_pagedefaults_default() {
        let via_builder = PageDefaults::builder().build();
        let via_default = PageDefaults::default();
        assert_eq!(via_builder.page_box, via_default.page_box);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p raikiri-traits pagedefaults_tests 2>&1 | tail -10`
Expected: FAIL — `PageDefaults` 未定義 (compile error)

- [ ] **Step 3: Implement `PageDefaults` + `PageDefaultsBuilder`**

`crates/raikiri-traits/src/page.rs` の末尾 (Task 2 で追記した const impl の後、`PageContext` struct の前 or ファイル最末尾) に追記:

```rust
/// Consumer が render 開始時に渡す page-level default 値。M1 は paper size
/// のみを持つ最小 shape。M2+ で margin / orientation / named pages 等を追加予定。
///
/// 全 field は CSS px 単位 (`PageBox` 参照)。pt/mm/in 換算は Consumer 責務。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PageDefaults {
    /// Default paper サイズ (`@page size` で override しない場合の initial value)。
    /// 既定 = A4。
    pub page_box: PageBox,
}

impl PageDefaults {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PageDefaultsBuilder {
        PageDefaultsBuilder::default()
    }
}

impl Default for PageDefaults {
    fn default() -> Self {
        Self {
            page_box: PageBox::default(),
        }
    }
}

/// `PageDefaults` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct PageDefaultsBuilder {
    page_box: Option<PageBox>,
}

impl PageDefaultsBuilder {
    /// `page_box` を設定。
    pub fn page_box(mut self, v: PageBox) -> Self {
        self.page_box = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PageDefaults {
        PageDefaults {
            page_box: self.page_box.unwrap_or_default(),
        }
    }
}
```

- [ ] **Step 4: Add to raikiri-traits lib.rs pub re-export**

`crates/raikiri-traits/src/lib.rs` の既存 `pub use page::{...}` を探し (現状 PageBox / PageContext / PageFragment / LayoutBuffer / TargetRegistry / RunningTemplate / FormData / GcpmDirective / ContentValueItem が並んでいる)、`PageDefaults, PageDefaultsBuilder,` を追加:

```rust
pub use page::{
    ContentValueItem, FormData, GcpmDirective, LayoutBuffer, PageBox, PageContext,
    PageDefaults, PageDefaultsBuilder,   // ← 追加
    PageFragment, RunningTemplate, TargetRegistry,
};
```

**注**: 既存 re-export の並びが alphabetical か semantic かは file 現状に合わせる。新規 2 items は近接配置。

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p raikiri-traits pagedefaults_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: Verify pub re-export**

Run: `cargo doc -p raikiri-traits --no-deps 2>&1 | grep -E "warning|error" | head -5`
Expected: (empty) or `warning: 0 warnings`

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri-traits/src/page.rs crates/raikiri-traits/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(raikiri-traits): PageDefaults + PageDefaultsBuilder を追加 (raikiri-spike-m1.11)

Consumer が render 開始時に渡す page-level default 値。M1 は page_box のみを
持つ最小 shape、M2+ で margin/orientation/named-pages を追加予定。
plan / render_streaming の第 2 引数として消費される。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 4: `HtmlDocument` struct + accessors

**Files:**
- Create: `crates/raikiri/src/html_document.rs`
- Modify: `crates/raikiri/src/lib.rs` (module 宣言 + pub re-export)

**Interfaces:**
- Consumes: `raikiri_html::UncascadedDocument`, `raikiri_style::CascadeResult`, `raikiri_dom::Document`
- Produces: `pub struct raikiri::HtmlDocument` with 3 accessors: `dom() -> &raikiri_dom::Document`, `cascade() -> &CascadeResult`, `stylesheet_sources() -> &[String]` (Task 5 parse_html が作成、Task 6 stubs が第 1 引数で受ける、Task 7 re-export、Task 8 integration test が消費)

- [ ] **Step 1: Write failing test (unit, crate-internal)**

`crates/raikiri/src/lib.rs` の既存 `#[cfg(test)] mod smoke_tests` を残しつつ、末尾に新規 mod を追加 (Task 5/6 tests もここに集約):

```rust
#[cfg(test)]
mod html_document_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = raikiri_html::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
        let cascade = build_cascaded(&uncascaded);
        // 内部 field 直接 construct (crate-internal test なので pub(crate) field OK)
        HtmlDocument { uncascaded, cascade }
    }

    #[test]
    fn html_document_accessors_expose_underlying_types() {
        let doc = hello_world_doc();
        // accessor が inner field と identity 一致 (別 heap 割当てなし)
        let dom_ref: &raikiri_dom::Document = doc.dom();
        let cascade_ref: &CascadeResult = doc.cascade();
        let sources_ref: &[String] = doc.stylesheet_sources();

        assert!(std::ptr::eq(dom_ref, &doc.uncascaded.dom),
            "dom() must return &doc.uncascaded.dom");
        assert!(std::ptr::eq(cascade_ref, &doc.cascade),
            "cascade() must return &doc.cascade");
        assert!(std::ptr::eq(sources_ref.as_ptr(), doc.uncascaded.stylesheet_sources.as_ptr())
            || (sources_ref.is_empty() && doc.uncascaded.stylesheet_sources.is_empty()),
            "stylesheet_sources() must alias inner Vec");
    }

    #[test]
    fn html_document_cascade_populated_after_construct() {
        let doc = hello_world_doc();
        assert!(!doc.cascade().computed.is_empty(),
            "cascade must be populated (build_cascaded produces per-node ComputedValues)");
        // node_count と cascade.computed.len() 契約
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade.computed.len() must equal document.node_count() (m1.23 contract)"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p raikiri html_document_tests 2>&1 | tail -10`
Expected: FAIL — `HtmlDocument` 未定義 (compile error)

- [ ] **Step 3: Create `crates/raikiri/src/html_document.rs`**

```rust
//! `HtmlDocument`: M1 assembled document type (raikiri-spike-m1.11)。
//!
//! HTML を parse して cascade まで完了した document unit。Consumer 視点で
//! 「layout/paint に投入できる状態」を単一 handle で表現する。M2+ で
//! raikiri-dom に `Document::assemble` が生えた時点で `pub use raikiri_dom::
//! Document` に透過的に置換される (Consumer surface 不変)。
//!
//! blitz `HtmlDocument` の analog (spec §L1134 blitz-compat 対応)。名前のみ
//! 一致、shape / code / UA CSS の持ち込みなし (memory
//! `raikiri-implementation-independence` 準拠)。

use raikiri_html::UncascadedDocument;
use raikiri_style::CascadeResult;

/// Cascade まで完了した document unit。
#[non_exhaustive]
pub struct HtmlDocument {
    pub(crate) uncascaded: UncascadedDocument,
    pub(crate) cascade: CascadeResult,
}

impl HtmlDocument {
    /// DOM tree への参照 (Dom / Element trait を使う際の entry point)。
    pub fn dom(&self) -> &raikiri_dom::Document {
        &self.uncascaded.dom
    }

    /// Cascade 結果 (per-node ComputedValues)。
    pub fn cascade(&self) -> &CascadeResult {
        &self.cascade
    }

    /// Parse 時に head 配下から集約された `<style>` element の source list
    /// (M1 契約、[`UncascadedDocument::stylesheet_sources`] に一致)。
    /// `<body>` 内 `<style>` は M1 未対応 (M2+ で拡張予定、m1.23 契約継承)。
    pub fn stylesheet_sources(&self) -> &[String] {
        &self.uncascaded.stylesheet_sources
    }
}
```

- [ ] **Step 4: Add module + re-export in `crates/raikiri/src/lib.rs`**

`crates/raikiri/src/lib.rs` の既存 `use raikiri_style::cascade;` 直後 (最初の use group の後) に:

```rust
mod html_document;
pub use html_document::HtmlDocument;
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p raikiri html_document_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 2 passed`

- [ ] **Step 6: Regression check**

Run: `cargo test -p raikiri 2>&1 | grep -E "^test result:"`
Expected: 既存 test 全 pass + 新 test 2 pass

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri/src/html_document.rs crates/raikiri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(raikiri): HtmlDocument struct + accessor 3 method を追加 (raikiri-spike-m1.11)

M1 assembled document type。UncascadedDocument + CascadeResult の私設 wrapper
として実装、M2+ で raikiri_dom::Document::assemble に透過的に付け替え可能な
surface。dom() / cascade() / stylesheet_sources() の 3 accessor で下位型を露出。
blitz HtmlDocument の analog (名前のみ一致、shape は raikiri 独自)。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 5: `parse_html<R: Read>` orchestrator

**Files:**
- Create: `crates/raikiri/src/parse.rs`
- Modify: `crates/raikiri/src/lib.rs` (module 宣言 + pub re-export)

**Interfaces:**
- Consumes: `raikiri_html::parse` (raikiri-html), `build_cascaded` (lib.rs, m1.23), `HtmlDocument` (Task 4)
- Produces: `pub fn raikiri::parse_html<R: Read>(input: R, options: &ParseOptions<'_>) -> Result<HtmlDocument, RenderError>` (Task 8 integration test が消費、m1.14 hello-world-vrt が消費)

- [ ] **Step 1: Write failing tests**

`crates/raikiri/src/lib.rs` の `html_document_tests` mod の直後に:

```rust
#[cfg(test)]
mod parse_html_tests {
    use super::*;

    #[test]
    fn parse_html_returns_html_document_with_cascade_populated() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html should succeed");
        assert!(!doc.cascade().computed.is_empty(),
            "parse_html output must have populated cascade");
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade / dom node_count invariant"
        );
    }

    #[test]
    fn parse_html_propagates_parse_error_from_io() {
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
            }
        }
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err = parse_html(FailingReader, &opts).expect_err("must fail on reader error");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Io(_))),
            "expected RenderError::Parse(ParseError::Io), got {err:?}"
        );
    }

    #[test]
    fn parse_html_propagates_utf8_error() {
        // 0x80 は UTF-8 continuation byte 単独、invalid UTF-8
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err = parse_html(&[0x80u8, 0x80, 0x80][..], &opts)
            .expect_err("must fail on invalid UTF-8");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Encoding { .. })),
            "expected RenderError::Parse(ParseError::Encoding), got {err:?}"
        );
    }

    #[test]
    fn parse_html_baked_cascade_matches_manual_build_cascaded() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // 2 経路の cascade が同じ結果を出すことを pin (parse_html は
        // build_cascaded を内部で呼んでいる契約)
        let via_parse_html = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");
        let via_manual = {
            let uncascaded = raikiri_html::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
            build_cascaded(&uncascaded)
        };
        assert_eq!(
            via_parse_html.cascade().computed.len(),
            via_manual.computed.len(),
            "parse_html と手動 build_cascaded で cascade node 数が一致"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p raikiri parse_html_tests 2>&1 | tail -10`
Expected: FAIL — `parse_html` 未定義 (compile error)

- [ ] **Step 3: Create `crates/raikiri/src/parse.rs`**

```rust
//! `parse_html`: HTML byte stream から cascade 済 [`HtmlDocument`] を生成する
//! orchestrator (raikiri-spike-m1.11)。
//!
//! spec §L1060 の pub API 相当。内部 pipeline は
//! `raikiri_html::parse` → [`build_cascaded`] → assemble。M1 では cascade は
//! 常に `Ok` を返すため、`RenderError::Parse` のみが bubble する。

use std::io::Read;

use raikiri_html::ParseOptions;
use raikiri_traits::RenderError;

use crate::{HtmlDocument, build_cascaded};

/// HTML byte stream を parse し、cascade まで完了した [`HtmlDocument`] を返す。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
///
/// M1 では cascade は infallible (m1.23 契約、`.expect` で unwrap)。
///
/// # Example
///
/// ```
/// use raikiri::{parse_html, ParseOptions};
///
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
/// assert!(!doc.cascade().computed.is_empty());
/// ```
pub fn parse_html<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<HtmlDocument, RenderError> {
    let uncascaded = raikiri_html::parse(input, options).map_err(RenderError::Parse)?;
    let cascade = build_cascaded(&uncascaded);
    Ok(HtmlDocument { uncascaded, cascade })
}
```

- [ ] **Step 4: Add module + re-export in `crates/raikiri/src/lib.rs`**

Task 4 で追加した `mod html_document;` の直後に:

```rust
mod parse;
pub use parse::parse_html;
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p raikiri parse_html_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: Regression check**

Run: `cargo test -p raikiri 2>&1 | grep -E "^test result:"`
Expected: 既存 + Task 4 + Task 5 の test 全 pass

- [ ] **Step 7: Commit**

```bash
git add crates/raikiri/src/parse.rs crates/raikiri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(raikiri): parse_html orchestrator を追加 (raikiri-spike-m1.11)

HTML byte stream を parse → build_cascaded → HtmlDocument assemble の 1 shot
pipeline を pub API として提供。ParseError は RenderError::Parse で bubble、
cascade は M1 infallible 契約継承 (m1.23)。spec §L1060 の signature 準拠。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 6: `plan` / `render_streaming` stubs

**Files:**
- Create: `crates/raikiri/src/stubs.rs`
- Modify: `crates/raikiri/src/lib.rs` (module 宣言 + pub re-export、raikiri-traits の追加 re-export)

**Interfaces:**
- Consumes: `HtmlDocument` (Task 4), `PageDefaults` (Task 3), `RenderError::Unimplemented` (Task 1), `PlanConfig`/`StreamingConfig`/`DocumentPlan`/`RenderStatus`/`ReplacedResolver`/`RenderSink` (raikiri-traits 既存、Task 7 で raikiri crate 経由 re-export)
- Produces: `pub fn plan(...)` / `pub fn render_streaming(...)` (M2+ で本実装差し替え、Task 8 integration test が Err assert)

- [ ] **Step 1: Add prerequisite re-exports to lib.rs (stub が pattern-match するため)**

`crates/raikiri/src/lib.rs` の `pub use raikiri_traits::{...}` block に **plan/render_streaming の signature に必要な最小限を先出しで追加** (Task 7 で残りを一括追加):

現状:
```rust
pub use raikiri_traits::{
    AbortController, AbortSignal, Body, CascadeError, Dom, Element, FetchedResource, HeaderMap,
    Method, NetworkError, NetworkProvider, Node, NodeId, NodeKind, ParseError, QuirksMode,
    RenderError, RenderWarning, Request, ResourceKind, StylesheetKind,
};
```

を以下に置換 (Task 6 用の最小追加、Task 7 で残り追加):

```rust
pub use raikiri_traits::{
    AbortController, AbortSignal, Body, CascadeError, Dom, Element, FetchedResource, HeaderMap,
    Method, NetworkError, NetworkProvider, Node, NodeId, NodeKind, ParseError, QuirksMode,
    RenderError, RenderWarning, Request, ResourceKind, StylesheetKind,
    // Task 6 (stub) 用の最小追加:
    DocumentPlan, PageDefaults, PlanConfig, RenderSink, RenderStatus, ReplacedResolver,
    StreamingConfig,
};
```

- [ ] **Step 2: Write failing tests**

`crates/raikiri/src/lib.rs` の `parse_html_tests` mod の直後に:

```rust
#[cfg(test)]
mod stub_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse")
    }

    /// M1 では replaced element なし → resolve が呼ばれない前提で unreachable。
    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(
            &self,
            _req: &raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            unreachable!("plan/render_streaming stubs must not call resolver")
        }
    }

    /// M1 sink stub。accept_page / finish_render は No-op。stub は sink を呼ばない前提。
    struct NoopSink;
    impl RenderSink for NoopSink {
        fn accept_page(
            &mut self,
            _fragment: raikiri_traits::PageFragment,
        ) -> Result<(), std::io::Error> {
            unreachable!("render_streaming stub must not call sink")
        }
        fn finish_render(
            &mut self,
            _summary: &raikiri_traits::RenderSummary,
        ) -> Result<(), std::io::Error> {
            unreachable!("render_streaming stub must not call sink")
        }
    }

    #[test]
    fn plan_returns_unimplemented_with_feature_name() {
        let doc = hello_world_doc();
        let err = plan(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            PlanConfig::default(),
        )
        .expect_err("plan stub must return Err");
        match err {
            RenderError::Unimplemented { feature, .. } => {
                assert_eq!(feature, "plan", "feature must identify plan API");
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn render_streaming_returns_unimplemented_with_feature_name() {
        let doc = hello_world_doc();
        let mut sink = NoopSink;
        let err = render_streaming(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            StreamingConfig::default(),
            &mut sink,
        )
        .expect_err("render_streaming stub must return Err");
        match err {
            RenderError::Unimplemented { feature, migration_hint } => {
                assert_eq!(feature, "render_streaming", "feature must identify API");
                assert!(
                    migration_hint.contains("html_to_png"),
                    "hint must point Consumer to html_to_png, got {migration_hint:?}"
                );
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p raikiri stub_tests 2>&1 | tail -10`
Expected: FAIL — `plan` / `render_streaming` / `PageDefaults` の一部が未定義 (compile error)

`PageDefaults` はまだ Task 3 済で existing、`plan` / `render_streaming` は本 Task で追加、Task 7 で残り re-export。**Step 1 の最小 re-export により compile 通過 → 未定義 fn error のみ残る**。

- [ ] **Step 4: Create `crates/raikiri/src/stubs.rs`**

```rust
//! `plan` / `render_streaming` の M1 stub 実装 (raikiri-spike-m1.11)。
//!
//! この module 内の全 fn は `RenderError::Unimplemented` を返す。
//! **M2+ pagination 実装完了時に、本 module を丸ごと削除して parse.rs / dedicated
//! plan.rs / render_streaming.rs に本実装を配置し直す** (retire scope isolation)。
//!
//! - `plan`: M2+ pagination 完了後に populate、PlanConfig の initial_registry と
//!   併せて DocumentPlan を返す本実装に差し替え
//! - `render_streaming`: M2 pagestream-state-machine で本実装、`RenderSink` に
//!   PageFragment を stream 出力する
//!
//! M1 では両者とも Consumer に「html_to_png (m1.14) を使え」と migration hint
//! を返す。

use raikiri_traits::{
    DocumentPlan, PageDefaults, PlanConfig, RenderError, RenderSink, RenderStatus,
    ReplacedResolver, StreamingConfig,
};

use crate::HtmlDocument;

/// Plan mode (dry-run: parse+cascade+layout planning のみ、PaintedBox 構築なし)。
///
/// **M1 stub**: 常に `Err(RenderError::Unimplemented { feature: "plan", .. })` を
/// 返す。本実装は M2+ pagination 完了後。
///
/// spec §L1075 の signature 準拠。
pub fn plan(
    _doc: &HtmlDocument,
    _defaults: PageDefaults,
    _resolver: &dyn ReplacedResolver,
    _config: PlanConfig,
) -> Result<DocumentPlan, RenderError> {
    Err(RenderError::Unimplemented {
        feature: "plan",
        migration_hint: "M1 non-goal; M2+ で pagination 実装後に populate",
    })
}

/// Streaming rendering (1 pass、BoundedLookahead + PlaceholderTargetResolver +
/// ImmediateEmission)。
///
/// **M1 stub**: 常に `Err(RenderError::Unimplemented { feature: "render_streaming", .. })`
/// を返す。本実装は M2 pagestream-state-machine で。
///
/// spec §L1084 の signature 準拠。
pub fn render_streaming(
    _doc: &HtmlDocument,
    _defaults: PageDefaults,
    _resolver: &dyn ReplacedResolver,
    _config: StreamingConfig,
    _sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    Err(RenderError::Unimplemented {
        feature: "render_streaming",
        migration_hint: "M1 では html_to_png (m1.14) 経路のみ動作、render_streaming は M2 pagestream で実装",
    })
}
```

- [ ] **Step 5: Add module + re-export in `crates/raikiri/src/lib.rs`**

Task 5 で追加した `mod parse;` の直後に:

```rust
mod stubs;
pub use stubs::{plan, render_streaming};
```

- [ ] **Step 6: Run tests to verify pass**

Run: `cargo test -p raikiri stub_tests 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 2 passed`

- [ ] **Step 7: Regression check**

Run: `cargo test -p raikiri 2>&1 | grep -E "^test result:"`
Expected: 既存 + Task 4/5/6 の全 test pass

- [ ] **Step 8: Commit**

```bash
git add crates/raikiri/src/stubs.rs crates/raikiri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(raikiri): plan / render_streaming stub を追加 (raikiri-spike-m1.11)

M1 stub 実装。両者とも RenderError::Unimplemented { feature, migration_hint }
を返す。M2+ pagination 実装完了時に stubs.rs を削除、本実装 module に差し替え
る予定 (retire scope isolation for grep visibility)。spec §L1075 / §L1084 の
signature 準拠。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 7: Full re-export additions in raikiri crate lib.rs

**Files:**
- Modify: `crates/raikiri/src/lib.rs` (`pub use raikiri_traits::{...}` を全 type に拡張)

**Interfaces:**
- Consumes: raikiri-traits の全 pub type
- Produces: raikiri crate から `use raikiri::*;` で全 type にアクセス可能 (Task 8 integration test、m1.14 / m1.15 の下流 task が消費)

- [ ] **Step 1: Write failing test — external consumer compile check の scaffolding**

`crates/raikiri/tests/external_consumer.rs` を新規作成 (Task 8 で本充実、ここでは compile 検証用の minimal test 1 個):

```rust
//! External consumer が `use raikiri::*;` のみで parse_html → plan (Err) →
//! render_streaming (Err) の chain を書けることを compile + run で pin
//! (raikiri-spike-m1.15 前哨、raikiri-spike-m1.11 で追加)。

use raikiri::*;

#[test]
fn external_consumer_can_reference_all_reexported_types() {
    // 各型が use raikiri::*; だけで名前解決できることを compile で pin。
    // 実際に値を使う必要なし (dead_code lint 抑制のため let _ で消費)。
    let _ = std::marker::PhantomData::<(
        RenderStatus,
        RenderSummary,
        LimitKind,
        DocumentPlan,
        PageSummary,
        PageDefaults,
        PageDefaultsBuilder,
        PageBox,
        PageContext,
        PageFragment,
        PlanConfig,
        PlanConfigBuilder,
        StreamingConfig,
        StreamingConfigBuilder,
        BatchConfig,
        BatchConfigBuilder,
        LookaheadConfig,
        LookaheadConfigBuilder,
        RenderLimits,
        RenderLimitsBuilder,
        LayoutError,
        Symbol,
    )>::default();
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p raikiri --test external_consumer 2>&1 | tail -20`
Expected: FAIL — 多くの型が raikiri から re-export されていない (compile error `no PageSummary in raikiri` 等)

- [ ] **Step 3: Expand `pub use raikiri_traits::{...}` in lib.rs**

`crates/raikiri/src/lib.rs` の Task 6 で拡張した `pub use raikiri_traits::{...}` block を以下の完全 list に置換:

```rust
pub use raikiri_traits::{
    // ── 既存 (m1.23) ──
    AbortController, AbortSignal, Body, CascadeError, Dom, Element,
    FetchedResource, HeaderMap, Method, NetworkError, NetworkProvider,
    Node, NodeId, NodeKind, ParseError, QuirksMode, RenderError, RenderWarning,
    Request, ResourceKind, StylesheetKind,

    // ── error / status 系 ──
    RenderStatus, RenderSummary, LimitKind, UnresolvedTarget, UnresolvedReason,
    EmittedSlotInfo, TargetSlotId, TargetKind, TargetDiscrepancy, ExhaustionPolicy,

    // ── plan mode ──
    DocumentPlan, PageSummary, BreakReason, TargetDefinition,

    // ── config ──
    RenderLimits, RenderLimitsBuilder,
    LookaheadConfig, LookaheadConfigBuilder,
    PlanConfig, PlanConfigBuilder,
    StreamingConfig, StreamingConfigBuilder,
    BatchConfig, BatchConfigBuilder,

    // ── paged model ──
    PageBox, PageContext, PageFragment,
    PageDefaults, PageDefaultsBuilder,
    LayoutBuffer, TargetRegistry, RunningTemplate, FormData,
    GcpmDirective, ContentValueItem,

    // ── traits (Consumer が implement) ──
    RenderSink, ReplacedResolver, ResourcePolicy,

    // ── strategy traits ──
    LookaheadPolicy, TargetResolver, EmissionPolicy, ReflowPolicy,
    ReflowAction, ContainerOverflowFallback, DirtyDeadline,
    ProbeContext, TargetRequest, ResolvedTarget,

    // ── resolver 補助 ──
    IntrinsicBox, ResolvedIntrinsic, ResolveDisposition,
    ResolverRequest, ResolverError,

    // ── policy 補助 ──
    PolicyViolation, ViolationType,

    // ── layout 補助 ──
    LayoutError,

    // ── symbol ──
    Symbol,
};
```

**注**: Task 6 で先出し追加した `DocumentPlan, PageDefaults, PlanConfig, RenderSink, RenderStatus, ReplacedResolver, StreamingConfig` は本 block 内に包含される (重複しないよう最終形は 1 箇所のみ)。

- [ ] **Step 4: Run test to verify pass**

Run: `cargo test -p raikiri --test external_consumer 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 1 passed`

- [ ] **Step 5: rustdoc regression check**

Run: `cargo doc -p raikiri --no-deps 2>&1 | grep -E "warning|error" | head -10`
Expected: (empty)

- [ ] **Step 6: Commit**

```bash
git add crates/raikiri/src/lib.rs crates/raikiri/tests/external_consumer.rs
git commit -m "$(cat <<'EOF'
feat(raikiri): raikiri-traits の全 pub type を umbrella から re-export (raikiri-spike-m1.11)

m1.14 hello-world VRT / m1.15 external consumer compile test が use raikiri::*;
のみで完結する API surface を確立。error/status/plan/config/page/traits/strategy
/resolver/policy/layout/symbol の 10 カテゴリ計 40+ 型を追加 re-export。
compile 検証 test を tests/external_consumer.rs に scaffolding として配置。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 8: Integration tests (external_consumer.rs 充実)

**Files:**
- Modify: `crates/raikiri/tests/external_consumer.rs` (Task 7 で作成した scaffolding に 3 tests 追加)

**Interfaces:**
- Consumes: raikiri crate の全 pub API (Task 4-7 で追加)
- Produces: integration test 3 個 (external consumer 視点、`use raikiri::*` のみ)

- [ ] **Step 1: Add failing tests**

`crates/raikiri/tests/external_consumer.rs` に、既存の `external_consumer_can_reference_all_reexported_types` テストの下に追記:

```rust
/// External consumer が `use raikiri::*;` のみで parse_html → plan (Err) →
/// render_streaming (Err) の chain を書けることを compile + run で pin。
/// (design test #9)
#[test]
fn external_consumer_can_call_parse_plan_render_streaming() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");

    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(
            &self,
            _req: &ResolverRequest<'_>,
        ) -> Result<ResolvedIntrinsic, ResolverError> {
            unreachable!()
        }
    }
    struct NoopSink;
    impl RenderSink for NoopSink {
        fn accept_page(&mut self, _f: PageFragment) -> Result<(), std::io::Error> {
            unreachable!()
        }
        fn finish_render(&mut self, _s: &RenderSummary) -> Result<(), std::io::Error> {
            unreachable!()
        }
    }

    let plan_err = plan(&doc, PageDefaults::default(), &NoopResolver, PlanConfig::default())
        .expect_err("plan stub must Err");
    assert!(matches!(plan_err, RenderError::Unimplemented { feature: "plan", .. }));

    let mut sink = NoopSink;
    let stream_err = render_streaming(
        &doc,
        PageDefaults::default(),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect_err("render_streaming stub must Err");
    assert!(matches!(
        stream_err,
        RenderError::Unimplemented { feature: "render_streaming", .. }
    ));
}

/// 全 `#[non_exhaustive]` struct が external crate から X::default() / builder
/// で constructable なことを pin (m1.15 acceptance criteria 前哨、design test #10)。
#[test]
fn external_consumer_can_construct_all_non_exhaustive_types() {
    // struct via Default
    let _ = PageDefaults::default();
    let _ = PageBox::default();
    let _ = PageContext::default();
    let _ = PageFragment::default();
    let _ = PlanConfig::default();
    let _ = StreamingConfig::default();
    let _ = BatchConfig::default();
    let _ = LookaheadConfig::default();
    let _ = RenderLimits::default();

    // struct via builder
    let _ = PageDefaults::builder().build();
    let _ = PlanConfig::builder().build();
    let _ = StreamingConfig::builder().build();
    let _ = BatchConfig::builder().build();
    let _ = LookaheadConfig::builder().build();
    let _ = RenderLimits::builder().build();

    // HtmlDocument は parse_html を経由 (private field なので direct construct 不可、
    // これが M2+ で pub use raikiri_dom::Document に置換した際にも同じ制約)
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let _doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
}

/// PageBox の px 単位切替の regression pin (design test #11)。
#[test]
fn pagedefaults_us_letter_and_a4_have_expected_px_values() {
    assert!(
        (PageBox::A4.width - 793.7008).abs() < 0.001,
        "A4.width = 793.7008 px"
    );
    assert!(
        (PageBox::A4.height - 1122.5197).abs() < 0.001,
        "A4.height = 1122.5197 px"
    );
    assert_eq!(PageBox::US_LETTER.width, 816.0);
    assert_eq!(PageBox::US_LETTER.height, 1056.0);

    // PageDefaults の default paper が A4 であることも pin
    assert_eq!(PageDefaults::default().page_box, PageBox::A4);
}
```

- [ ] **Step 2: Run tests to verify they pass**

Task 1-7 が正しく完了していれば追加 3 tests は最初から pass (integration test の目的は compile + 契約 pin であり、実装が正しく揃っていれば failing test cycle は無し)。

Run: `cargo test -p raikiri --test external_consumer 2>&1 | grep -E "^test result:"`
Expected: `test result: ok. 4 passed` (Task 7 の 1 + 本 Task の 3)

- [ ] **Step 3: Run full raikiri test suite**

Run: `cargo test -p raikiri 2>&1 | grep -E "^test result:"`
Expected: 全 test suite `ok` (smoke_tests + build_cascaded + html_document_tests + parse_html_tests + stub_tests + external_consumer の 6 groups)

- [ ] **Step 4: Commit**

```bash
git add crates/raikiri/tests/external_consumer.rs
git commit -m "$(cat <<'EOF'
test(raikiri): external consumer integration test 3 個を追加 (raikiri-spike-m1.11)

design §6 test #9-11 を実装。use raikiri::*; のみで parse_html → plan (Err) →
render_streaming (Err) の chain を pin、全 #[non_exhaustive] 型が
Default/builder で constructable なことを pin (m1.15 acceptance 前哨)、
PageBox の px 単位切替 (A4/US_LETTER const 値) を regression pin。

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Task 9: Final workspace verification (clippy + doc + full test)

**Files:**
- (none — verification only)

**Interfaces:**
- Consumes: Task 1-8 の全成果
- Produces: acceptance criteria 全達成の確認 evidence

- [ ] **Step 1: Full workspace test**

Run: `cargo test --workspace 2>&1 | tee /tmp/m1.11-final-test.log | grep -E "FAIL|^test result:" | head -30`
Expected: 全 test suite `ok`、0 failures

Fail 時は log full を確認して原因 fix、該当 task に戻る。

- [ ] **Step 2: Full workspace build**

Run: `cargo build --workspace 2>&1 | tail -10`
Expected: `Finished dev [unoptimized + debuginfo] target(s) in ...s`、warnings なし

- [ ] **Step 3: Workspace clippy**

Run: `cargo clippy --workspace --all-targets 2>&1 | tail -20`
Expected: warnings なし、`Finished` で終了

warning あり → 該当 task に戻り clippy suggestion に追随。

- [ ] **Step 4: Rustdoc build**

Run: `cargo doc -p raikiri -p raikiri-traits --no-deps 2>&1 | grep -E "warning|error"`
Expected: (empty) — broken intra-doc link なし

- [ ] **Step 5: Acceptance criteria check-off**

`bd show raikiri-spike-m1.11` の `acceptance_criteria` を再読し、以下を確認:

- [x] `raikiri::HtmlDocument` struct + accessor 3 method → Task 4
- [x] `raikiri::parse_html<R: Read>` → Task 5
- [x] `raikiri::plan` stub Err(Unimplemented{feature:"plan"}) → Task 6
- [x] `raikiri::render_streaming` stub Err(Unimplemented{feature:"render_streaming"}) → Task 6
- [x] `raikiri-traits::PageDefaults` + Builder + re-export → Task 3
- [x] `PageBox` doc pt→px、A4 値更新、US_LETTER const → Task 2
- [x] `RenderError::Unimplemented { feature, migration_hint }` variant + Display/Error::source → Task 1
- [x] Design §5 re-export list 全型が use raikiri::*; で参照可能 → Task 7 + Task 8 test #1
- [x] Design §6 の 11 tests 全 pass → Task 4/5/6 unit + Task 8 integration (計 11)
- [x] File layout: src/{html_document,parse,stubs}.rs, lib.rs 集中管理 → Task 4/5/6
- [x] tests/external_consumer.rs に integration test 配置、build_cascaded.rs 変更なし → Task 7/8
- [x] `cargo build/test -p raikiri -p raikiri-traits` green → Step 1/2
- [x] `cargo clippy -p raikiri -p raikiri-traits` warning-free → Step 3
- [x] `cargo doc -p raikiri --no-deps` warning-free → Step 4
- [x] `raikiri-dom` / `raikiri-paint` 既存 test が PageBox 単位切替後も green → Task 2 Step 5/5b
- [x] blitz-html/blitz-dom を参照する code/doc なし → grep で確認
- [x] `raikiri_dom::Document` / `raikiri_style::CascadeResult` 直接 re-export 維持 → Task 7 で touch なし

- [ ] **Step 6: Cleanroom compliance check**

Run: `grep -rE "blitz_html|blitz_dom|blitz::" crates/raikiri crates/raikiri-traits 2>/dev/null | head -10`
Expected: (empty)

- [ ] **Step 7 (optional): Final aggregation commit**

Task 1-8 が全て個別 commit されているので、本 Task では追加 commit なし。

もし verification 段階で trivial fix (formatting, comment typo 等) が発生した場合のみ:

```bash
git add -p    # 変更差分を確認しながら stage
git commit -m "$(cat <<'EOF'
chore(raikiri): final polish per verification (raikiri-spike-m1.11)

<具体的な差分の要約>

Claude-Session: https://claude.ai/code/session_01Xzb6RqUhDy9rgT65ZF1R5V
EOF
)"
```

---

## Self-Review Checklist

Plan 完成後、以下を自身で確認:

**Spec 覆蓋:** design §1-8 の全 scope が Task 1-9 で cover されているか
- §1 Scope/Non-goals → Task 1-8 全体の scope
- §2 Public API surface → Task 4 (HtmlDocument), Task 5 (parse_html), Task 6 (stubs)
- §3 PageDefaults + PageBox px → Task 2 (PageBox), Task 3 (PageDefaults)
- §4 Unimplemented variant → Task 1
- §5 Re-export list → Task 7
- §6 Testing → Task 4/5/6 unit + Task 8 integration
- §7 File layout → Task 4/5/6 実装で file 分割
- §8 Deferred/rejected → 実装外 (design 記録のみ)

**Placeholder scan:** 全 Task の code block が完全 (TBD / TODO / similar to task N / write tests for above なし) — 確認済

**Type consistency:** Task 間で参照する signature 一致 — `HtmlDocument`, `PageDefaults`, `RenderError::Unimplemented { feature, migration_hint }` が全 Task で同じ shape で参照される — 確認済
