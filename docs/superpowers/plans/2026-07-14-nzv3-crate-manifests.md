# nzv.3 crate-manifests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** raikiri-spike の 10 crate 分の `Cargo.toml` と `src/lib.rs` stub を blitz-consistent な workspace 構成で production artifact として整備する。

**Architecture:** root `Cargo.toml` の `[workspace.dependencies]` に外部 dep / in-repo path 参照 / dev-only oracle を集中宣言。10 crate は spec §4 の依存 DAG に沿って `{ workspace = true }` で参照。各 crate の `[package]` は `edition` / `rust-version` / `license` を workspace 継承し、`[lints] workspace = true` で root の rust lint (`unsafe_code = "deny"`, `missing_docs = "warn"`) を継承する。

**Tech Stack:** Rust 1.85 (edition 2024, resolver = "3"), cargo workspace, blitz と揃えた external dep version (cssparser 0.37 / selectors 0.39 / html5ever 0.39 / markup5ever 0.39 / taffy 0.12 / parley 0.10 / anyrender 0.11 / peniko 0.6 / kurbo 0.13 / anyrender_vello_cpu 0.14 / blitz-{traits,html,dom,paint} 0.3.0-beta.1)。

## Global Constraints

- Rust edition **2024**、`rust-version = "1.85.0"` (root `[workspace.package]` から継承)。
- workspace resolver = **"3"** (root で設定済、変更しない)。
- 各 crate は `[lints] workspace = true` を持ち、root の `[workspace.lints.rust]` (`unsafe_code = "deny"`, `missing_docs = "warn"`) を継承する。
- 各 crate の initial `version` = **"0.1.0"**。
- external dep は spec §3 の list を **blitz と同じ version に揃える** (nzv.10 selectors-cssparser 版整合 verify のため)。
- workspace member の in-repo path 参照は `{ version = "0.1.0", path = "./crates/<name>" }` の path + version 併用 (blitz 準拠)。
- Cargo.lock 生成と dep resolve の verify は **nzv.4 (dependency-resolution) の scope**、本 plan には含めない。
- `[profile.*]` block と virtual `[package]` block は **M0 scope 外**、追加しない (spec §13 で言及なし)。
- 各 crate の `src/lib.rs` は M0 では stub only (`//! <name> — M0 stub, populated in M1+`)、実装は M1 以降の該当 task が埋める。
- `[dev-dependencies]` は `cargo check --workspace` の default target (lib + bin) では resolve されないため、raikiri-wpt / raikiri-vrt の dev-only oracle 依存を declare しても M0 acceptance には影響しない。

---

## File Structure

**Modify (root):**
- `Cargo.toml`: `[workspace] members` に 10 path 追加、placeholder comment block を削除、`[workspace.dependencies]` を Task 1 で新設し以降 task で in-repo entry を追加

**Create (crates):**
- `crates/raikiri-traits/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-style/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-net/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-dom/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-html/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-paint/{Cargo.toml, src/lib.rs}`
- `crates/raikiri/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-blitz-compat/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-wpt/{Cargo.toml, src/lib.rs}`
- `crates/raikiri-vrt/{Cargo.toml, src/lib.rs}`

**依存 DAG に沿った task 順**

```
Task 1: root [workspace.dependencies] (external + dev-only)
Task 2: raikiri-traits            (leaf, foundation)
Task 3: raikiri-style, raikiri-net (depend on traits)
Task 4: raikiri-dom, raikiri-html, raikiri-paint (depend on style/traits)
Task 5: raikiri (umbrella), raikiri-blitz-compat (depend on all 6 core)
Task 6: raikiri-wpt, raikiri-vrt (dev-only, workspace 全 10 揃う)
```

---

### Task 1: Root `[workspace.dependencies]` + placeholder comment 除去

**Files:**
- Modify: `Cargo.toml` (workspace root)

**Interfaces:**
- Consumes: 現行 root Cargo.toml (`[workspace] resolver = "3", members = []`、`[workspace.package]` edition/rust-version/license、`[workspace.lints.rust]` unsafe_code/missing_docs)
- Produces: 外部 dep + dev-only oracle を宣言した `[workspace.dependencies]` block。in-repo crate path 参照は Tasks 2-6 で incrementally 追加される。

- [ ] **Step 1: Baseline を確認 (現行 workspace が parse できる)**

Run: `cargo metadata --format-version=1 --no-deps`
Expected: `"packages":[], "workspace_members":[], "workspace_default_members":[]` を含む JSON、exit 0。

- [ ] **Step 2: root `Cargo.toml` を書き換える**

現行 `Cargo.toml` の内容:

```toml
[workspace]
resolver = "3"
members = []
# nzv.3 crate-manifests で埋める予定の crate 一覧 (spec §4 Workspace Layout):
# - crates/raikiri              (umbrella crate)
# - crates/raikiri-traits       (純粋 API + trait 定義)
# - crates/raikiri-style        (CSS parse + cascade + GCPM at-rule)
# - crates/raikiri-dom          (HTML5 DOM)
# - crates/raikiri-html         (html5ever + TreeSink wrap)
# - crates/raikiri-paint        (anyrender::PaintScene 実装)
# - crates/raikiri-net          (NoOp / SandboxedNetProvider)
# - crates/raikiri-blitz-compat (blitz shape compat)
# - crates/raikiri-wpt          (WPT harness)
# - crates/raikiri-vrt          (VRT harness)

[workspace.package]
edition = "2024"
rust-version = "1.85.0"
license = "MIT OR Apache-2.0"

[workspace.lints.rust]
unsafe_code = "deny"
missing_docs = "warn"

[workspace.lints.clippy]
# M1 以降で必要に応じて拡張。M0 では空のまま resolver="3" 挙動の確認に集中
```

以下に置換 (placeholder comment block 削除、`[workspace.dependencies]` 新設):

```toml
[workspace]
resolver = "3"
members = []

[workspace.package]
edition = "2024"
rust-version = "1.85.0"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
# ── In-repo (populated per crate as they land in Tasks 2–6) ────────────

# ── Servo (cssparser + selectors) ──────────────────────────────────────
cssparser = "0.37"
selectors = "0.39"

# ── HTML5ever ──────────────────────────────────────────────────────────
html5ever   = "0.39"
markup5ever = "0.39"

# ── Layout (taffy) ─────────────────────────────────────────────────────
taffy = { version = "0.12", default-features = false, features = [
    "std",
    "flexbox",
    "grid",
    "block_layout",
    "content_size",
    "calc",
] }

# ── Text (parley) ──────────────────────────────────────────────────────
parley = { version = "0.10", default-features = false, features = ["std"] }

# ── Rendering (anyrender + peniko + kurbo) ─────────────────────────────
anyrender = "0.11"
peniko    = "0.6"
kurbo     = "0.13"

# ── Utility crates ─────────────────────────────────────────────────────
smol_str   = "0.3"
rustc-hash = "2.1"
bytes      = "1"
url        = "2.5"

# ── Dev-only: VRT rasterizer / snapshot / oracle ───────────────────────
anyrender_vello_cpu = { version = "0.14", features = ["multithreading"] }
tiny-skia           = "0.11"
insta               = "1"
blitz-traits        = "=0.3.0-beta.1"
blitz-html          = "=0.3.0-beta.1"
blitz-dom           = "=0.3.0-beta.1"
blitz-paint         = "=0.3.0-beta.1"

[workspace.lints.rust]
unsafe_code = "deny"
missing_docs = "warn"

[workspace.lints.clippy]
# M1 以降で必要に応じて拡張。M0 では空のまま resolver="3" 挙動の確認に集中
```

- [ ] **Step 3: TOML parse を検証**

Run: `cargo metadata --format-version=1 --no-deps`
Expected: 前と同じ空 workspace JSON、exit 0。`workspace.dependencies` に entry を積んでも member から参照されない間は resolve 走らず parse OK。

- [ ] **Step 4: commit**

```bash
git add Cargo.toml
git commit -m "build: populate workspace.dependencies for external and dev-only crates (M0/nzv.3 Task 1)"
```

---

### Task 2: `raikiri-traits` crate (foundation)

**Files:**
- Create: `crates/raikiri-traits/Cargo.toml`
- Create: `crates/raikiri-traits/src/lib.rs`
- Modify: `Cargo.toml` (root: `members` に追加、in-repo `workspace.dependencies` entry 追加)

**Interfaces:**
- Consumes: Task 1 が populate した `[workspace.dependencies]` から `smol_str`, `bytes`, `url`, `rustc-hash` を利用
- Produces: workspace member `raikiri-traits`。他 crate は root `workspace.dependencies` の `raikiri-traits = { version = "0.1.0", path = "./crates/raikiri-traits" }` 経由で参照する。

- [ ] **Step 1: baseline を確認 (empty workspace)**

Run: `cargo check --workspace`
Expected: `Finished` (0 crate、compile なし)、exit 0。

- [ ] **Step 2: `crates/raikiri-traits/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-traits"
version = "0.1.0"
description = "Foundation traits and neutral model types for raikiri (RenderSink, ReplacedResolver, NetworkProvider, strategy traits, PageFragment/PageBox/PageContext, etc.)"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
smol_str   = { workspace = true }
bytes      = { workspace = true }
url        = { workspace = true }
rustc-hash = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: `crates/raikiri-traits/src/lib.rs` を作成**

```rust
//! raikiri-traits — foundation traits and neutral model types.
//!
//! M0 stub. Full trait / type surface (spec §4 raikiri-traits) is populated
//! in M1 traits-definition + error-taxonomy-types tasks.
```

- [ ] **Step 4: root `Cargo.toml` の `[workspace] members` に追加**

```toml
[workspace]
resolver = "3"
members = ["crates/raikiri-traits"]
```

- [ ] **Step 5: root `Cargo.toml` の `[workspace.dependencies]` の in-repo section に entry 追加**

Task 1 で空にしていた `# ── In-repo (populated per crate as they land in Tasks 2–6) ─────` の直下に以下を追加:

```toml
raikiri-traits = { version = "0.1.0", path = "./crates/raikiri-traits" }
```

- [ ] **Step 6: `cargo check -p raikiri-traits` で検証**

Run: `cargo check -p raikiri-traits`
Expected: `Finished` (raikiri-traits と smol_str / bytes / url / rustc-hash とその推移 dep が resolve + compile 済み)、exit 0。

- [ ] **Step 7: commit**

```bash
git add crates/raikiri-traits Cargo.toml
git commit -m "build: add raikiri-traits crate stub (M0/nzv.3 Task 2)"
```

---

### Task 3: `raikiri-style` + `raikiri-net` (traits に依存する leaf 2 crate)

**Files:**
- Create: `crates/raikiri-style/{Cargo.toml, src/lib.rs}`
- Create: `crates/raikiri-net/{Cargo.toml, src/lib.rs}`
- Modify: root `Cargo.toml` (`members` に 2 entry 追加、`workspace.dependencies` の in-repo section に 2 entry 追加)

**Interfaces:**
- Consumes: `raikiri-traits.workspace = true`、`cssparser` + `selectors` (style)、`url` (net)
- Produces: workspace member `raikiri-style` と `raikiri-net`。後続の dom / html / umbrella が参照する。

- [ ] **Step 1: baseline を確認**

Run: `cargo check --workspace`
Expected: raikiri-traits のみ compile、exit 0。

- [ ] **Step 2: `crates/raikiri-style/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-style"
version = "0.1.0"
description = "CSS engine for raikiri: cssparser + selectors integration, unified RuleTree (@page / counter / running / target-*), cascade → ComputedValues, GCPM static-side directive emit"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
cssparser      = { workspace = true }
selectors      = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: `crates/raikiri-style/src/lib.rs` を作成**

```rust
//! raikiri-style — CSS engine (cssparser + selectors + cascade + GCPM static side).
//!
//! M0 stub. Populated in M1 css-cascade-basic and later milestones.
```

- [ ] **Step 4: `crates/raikiri-net/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-net"
version = "0.1.0"
description = "NetworkProvider / ReplacedResolver base implementations for raikiri (NoOpNetworkProvider, SandboxedNetProvider<P>, SandboxedResolver<R>, DenyAllPolicy, DefaultSandboxPolicy)"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
url            = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 5: `crates/raikiri-net/src/lib.rs` を作成**

```rust
//! raikiri-net — NetworkProvider / ReplacedResolver base implementations.
//!
//! M0 stub. Populated in M4 sandboxed-net-provider-impl and related tasks.
```

- [ ] **Step 6: root `Cargo.toml` の `[workspace] members` を更新**

`members` を以下に置換:

```toml
members = [
    "crates/raikiri-traits",
    "crates/raikiri-style",
    "crates/raikiri-net",
]
```

- [ ] **Step 7: root `Cargo.toml` の `[workspace.dependencies]` の in-repo section に entry 追加**

既存の `raikiri-traits` entry の後に追加:

```toml
raikiri-style = { version = "0.1.0", path = "./crates/raikiri-style" }
raikiri-net   = { version = "0.1.0", path = "./crates/raikiri-net" }
```

- [ ] **Step 8: 検証**

Run: `cargo check -p raikiri-style -p raikiri-net`
Expected: 2 crate + cssparser / selectors / url とその推移 dep が resolve + compile、exit 0。cssparser と selectors の major version が互換であることも同時に検証される (blitz と揃えた 0.37 + 0.39 が resolve できれば OK)。

- [ ] **Step 9: commit**

```bash
git add crates/raikiri-style crates/raikiri-net Cargo.toml
git commit -m "build: add raikiri-style and raikiri-net crate stubs (M0/nzv.3 Task 3)"
```

---

### Task 4: `raikiri-dom` + `raikiri-html` + `raikiri-paint` (mid-tier 3 crate)

**Files:**
- Create: `crates/raikiri-dom/{Cargo.toml, src/lib.rs}`
- Create: `crates/raikiri-html/{Cargo.toml, src/lib.rs}`
- Create: `crates/raikiri-paint/{Cargo.toml, src/lib.rs}`
- Modify: root `Cargo.toml` (`members` に 3 entry、`workspace.dependencies` の in-repo に 3 entry)

**Interfaces:**
- Consumes: `raikiri-traits.workspace = true` (全 crate)、`raikiri-style.workspace = true` (dom)、`raikiri-dom.workspace = true` (html)、`taffy` + `parley` (dom)、`html5ever` + `markup5ever` (html)、`anyrender` + `peniko` + `kurbo` (paint)
- Produces: workspace member `raikiri-dom` / `raikiri-html` / `raikiri-paint`。umbrella `raikiri` と dev crate から参照される。

- [ ] **Step 1: baseline を確認**

Run: `cargo check --workspace`
Expected: raikiri-traits / raikiri-style / raikiri-net の 3 crate compile、exit 0。

- [ ] **Step 2: `crates/raikiri-dom/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-dom"
version = "0.1.0"
description = "DOM data model + taffy/parley layout engine + PageStream/LayoutBuffer + GCPM runtime side (PageContext, TargetRegistry, PageBoxCache) for raikiri"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
raikiri-style  = { workspace = true }
taffy          = { workspace = true }
parley         = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: `crates/raikiri-dom/src/lib.rs` を作成**

```rust
//! raikiri-dom — DOM data model, layout engine (taffy + parley), and GCPM runtime side.
//!
//! M0 stub. Populated across M1 (dom-model, layout-single-page) through M5
//! (GCPM directive full) and M7 (break policy + probe layout).
```

- [ ] **Step 4: `crates/raikiri-html/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-html"
version = "0.1.0"
description = "Thin html5ever wrapper (RaikiriTreeSink) that produces UncascadedDocument for raikiri"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
raikiri-dom    = { workspace = true }
html5ever      = { workspace = true }
markup5ever    = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 5: `crates/raikiri-html/src/lib.rs` を作成**

```rust
//! raikiri-html — thin html5ever wrapper (RaikiriTreeSink, UncascadedDocument).
//!
//! M0 stub. Populated in M1 html-parse-basic task.
```

- [ ] **Step 6: `crates/raikiri-paint/Cargo.toml` を作成**

```toml
[package]
name = "raikiri-paint"
version = "0.1.0"
description = "PageFragment → anyrender::PaintScene walker for raikiri (VRT and debug output)"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
anyrender      = { workspace = true }
peniko         = { workspace = true }
kurbo          = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 7: `crates/raikiri-paint/src/lib.rs` を作成**

```rust
//! raikiri-paint — PageFragment → anyrender::PaintScene walker.
//!
//! M0 stub. Populated in M1 paint-basic task.
```

- [ ] **Step 8: root `Cargo.toml` の `[workspace] members` を更新**

```toml
members = [
    "crates/raikiri-traits",
    "crates/raikiri-style",
    "crates/raikiri-net",
    "crates/raikiri-dom",
    "crates/raikiri-html",
    "crates/raikiri-paint",
]
```

- [ ] **Step 9: root `Cargo.toml` の `[workspace.dependencies]` に in-repo entry 追加**

既存の in-repo entry 群の後ろに追加:

```toml
raikiri-dom   = { version = "0.1.0", path = "./crates/raikiri-dom" }
raikiri-html  = { version = "0.1.0", path = "./crates/raikiri-html" }
raikiri-paint = { version = "0.1.0", path = "./crates/raikiri-paint" }
```

- [ ] **Step 10: 検証**

Run: `cargo check -p raikiri-dom -p raikiri-html -p raikiri-paint`
Expected: 3 crate + taffy / parley / html5ever / markup5ever / anyrender / peniko / kurbo とその推移 dep が resolve + compile、exit 0。

- [ ] **Step 11: commit**

```bash
git add crates/raikiri-dom crates/raikiri-html crates/raikiri-paint Cargo.toml
git commit -m "build: add raikiri-dom, raikiri-html, raikiri-paint crate stubs (M0/nzv.3 Task 4)"
```

---

### Task 5: `raikiri` (umbrella) + `raikiri-blitz-compat`

**Files:**
- Create: `crates/raikiri/{Cargo.toml, src/lib.rs}`
- Create: `crates/raikiri-blitz-compat/{Cargo.toml, src/lib.rs}`
- Modify: root `Cargo.toml` (`members` に 2 entry、`workspace.dependencies` の in-repo に 2 entry)

**Interfaces:**
- Consumes: 上記 6 core crate (`raikiri-traits` / `raikiri-style` / `raikiri-net` / `raikiri-dom` / `raikiri-html` / `raikiri-paint`) を全て `{ workspace = true }` で参照
- Produces: workspace member `raikiri` (primary consumer API surface) と `raikiri-blitz-compat` (M6 で埋める stub)。blitz-compat は M6 で `blitz-traits` shape mirror を追加するまで raikiri しか参照しない (crates.io から blitz-traits を pull しない = M0 での version resolve risk を回避)。

- [ ] **Step 1: baseline を確認**

Run: `cargo check --workspace`
Expected: 既存 6 crate compile、exit 0。

- [ ] **Step 2: `crates/raikiri/Cargo.toml` を作成**

```toml
[package]
name = "raikiri"
version = "0.1.0"
description = "Umbrella crate for raikiri: primary consumer API (parse_html / plan / render_streaming / render_batch / html_to_png) and re-exports of all sub-crates"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri-traits = { workspace = true }
raikiri-style  = { workspace = true }
raikiri-net    = { workspace = true }
raikiri-dom    = { workspace = true }
raikiri-html   = { workspace = true }
raikiri-paint  = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: `crates/raikiri/src/lib.rs` を作成**

```rust
//! raikiri — umbrella crate: primary consumer API and re-exports.
//!
//! M0 stub. Populated in M1 umbrella-facade task (parse_html, plan,
//! render_streaming stubs) and later milestones.
```

- [ ] **Step 4: `crates/raikiri-blitz-compat/Cargo.toml` を作成**

M0 では blitz-traits shape mirror を含めない (M6 blitz-compat-type-shape task の scope)。raikiri のみに依存。

```toml
[package]
name = "raikiri-blitz-compat"
version = "0.1.0"
description = "blitz-compatible type shape layer for raikiri consumers (M6 で blitz_html::HtmlDocument / blitz_dom::Node compat を実装)"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
raikiri = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 5: `crates/raikiri-blitz-compat/src/lib.rs` を作成**

```rust
//! raikiri-blitz-compat — blitz-compatible type shape layer.
//!
//! M0 stub. Populated in M6e (blitz-compat-type-shape,
//! blitz-html-htmldocument-compat, blitz-dom-node-compat).
```

- [ ] **Step 6: root `Cargo.toml` の `[workspace] members` を更新**

```toml
members = [
    "crates/raikiri-traits",
    "crates/raikiri-style",
    "crates/raikiri-net",
    "crates/raikiri-dom",
    "crates/raikiri-html",
    "crates/raikiri-paint",
    "crates/raikiri",
    "crates/raikiri-blitz-compat",
]
```

- [ ] **Step 7: root `Cargo.toml` の `[workspace.dependencies]` in-repo に entry 追加**

```toml
raikiri              = { version = "0.1.0", path = "./crates/raikiri" }
raikiri-blitz-compat = { version = "0.1.0", path = "./crates/raikiri-blitz-compat" }
```

- [ ] **Step 8: 検証**

Run: `cargo check -p raikiri -p raikiri-blitz-compat`
Expected: 両 crate compile、追加の外部 dep なし (既に compile 済みの sub-crate のみ)、exit 0。

- [ ] **Step 9: commit**

```bash
git add crates/raikiri crates/raikiri-blitz-compat Cargo.toml
git commit -m "build: add raikiri umbrella and raikiri-blitz-compat crate stubs (M0/nzv.3 Task 5)"
```

---

### Task 6: `raikiri-wpt` + `raikiri-vrt` (dev-only、全 10 crate 揃う)

**Files:**
- Create: `crates/raikiri-wpt/{Cargo.toml, src/lib.rs}`
- Create: `crates/raikiri-vrt/{Cargo.toml, src/lib.rs}`
- Modify: root `Cargo.toml` (`members` に 2 entry、`workspace.dependencies` の in-repo に 2 entry)

**Interfaces:**
- Consumes: `raikiri.workspace = true` (両 crate)、`anyrender_vello_cpu.workspace = true` + `tiny-skia.workspace = true` (vrt: runtime dep)、`blitz-html` / `blitz-dom` / `blitz-paint` / `blitz-traits` / `insta` (wpt: dev-dependencies)、`insta` (vrt: dev-dependencies)
- Produces: workspace の最終 member 2 個、`cargo check --workspace` が全 10 crate + workspace 全体を pass する状態。

- [ ] **Step 1: baseline を確認**

Run: `cargo check --workspace`
Expected: 8 crate compile、exit 0。

- [ ] **Step 2: `crates/raikiri-wpt/Cargo.toml` を作成**

M0 stub なので runtime dep は raikiri のみ。dev-only oracle は `[dev-dependencies]` で declare (実際の resolve は M1 wpt-harness-skeleton 以降)。

```toml
[package]
name = "raikiri-wpt"
version = "0.1.0"
description = "WPT (web-platform-tests) runner harness for raikiri: executes W3C tests against raikiri with blitz-html/blitz-dom/blitz-paint as baseline oracle"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
raikiri = { workspace = true }

[dev-dependencies]
anyrender_vello_cpu = { workspace = true }
tiny-skia           = { workspace = true }
insta               = { workspace = true }
blitz-traits        = { workspace = true }
blitz-html          = { workspace = true }
blitz-dom           = { workspace = true }
blitz-paint         = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: `crates/raikiri-wpt/src/lib.rs` を作成**

```rust
//! raikiri-wpt — WPT harness with blitz oracle.
//!
//! M0 stub. Populated in M1 wpt-harness-skeleton and M8 wpt-full-sweep-scheduler / blitz-oracle-diff-recorder tasks.
```

- [ ] **Step 4: `crates/raikiri-vrt/Cargo.toml` を作成**

VRT はライブラリとして raikiri-wpt や integration test から使われる。runtime dep に anyrender_vello_cpu + tiny-skia が含まれる。

```toml
[package]
name = "raikiri-vrt"
version = "0.1.0"
description = "VRT (visual regression testing) harness for raikiri: anyrender_vello_cpu rasterizer wrapper + PNG diff"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
raikiri             = { workspace = true }
anyrender_vello_cpu = { workspace = true }
tiny-skia           = { workspace = true }

[dev-dependencies]
insta = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 5: `crates/raikiri-vrt/src/lib.rs` を作成**

```rust
//! raikiri-vrt — VRT harness (anyrender_vello_cpu rasterizer + PNG diff).
//!
//! M0 stub. Populated in M1 vrt-tiny-skia and hello-world-vrt tasks.
```

- [ ] **Step 6: root `Cargo.toml` の `[workspace] members` を最終形に**

```toml
members = [
    "crates/raikiri-traits",
    "crates/raikiri-style",
    "crates/raikiri-net",
    "crates/raikiri-dom",
    "crates/raikiri-html",
    "crates/raikiri-paint",
    "crates/raikiri",
    "crates/raikiri-blitz-compat",
    "crates/raikiri-wpt",
    "crates/raikiri-vrt",
]
```

- [ ] **Step 7: root `Cargo.toml` の `[workspace.dependencies]` in-repo に entry 追加**

```toml
raikiri-wpt = { version = "0.1.0", path = "./crates/raikiri-wpt" }
raikiri-vrt = { version = "0.1.0", path = "./crates/raikiri-vrt" }
```

- [ ] **Step 8: workspace 全体を検証**

Run: `cargo check --workspace`
Expected: 全 10 crate + runtime dep (anyrender_vello_cpu, tiny-skia を含む) が resolve + compile、error 0、exit 0。warning は許容 (missing_docs = "warn" の該当なし、unused import なし)。

- [ ] **Step 9: `cargo metadata` で最終 acceptance を確認**

Run: `cargo metadata --format-version=1 --no-deps | python3 -c "import json,sys; m=json.load(sys.stdin); print('members:', len(m['workspace_members'])); print('names:', sorted(p['name'] for p in m['packages']))"`
Expected:
```
members: 10
names: ['raikiri', 'raikiri-blitz-compat', 'raikiri-dom', 'raikiri-html', 'raikiri-net', 'raikiri-paint', 'raikiri-style', 'raikiri-traits', 'raikiri-vrt', 'raikiri-wpt']
```

- [ ] **Step 10: commit**

```bash
git add crates/raikiri-wpt crates/raikiri-vrt Cargo.toml
git commit -m "build: add raikiri-wpt and raikiri-vrt dev crate stubs (M0/nzv.3 Task 6)"
```

---

## Contingency: dep version が resolve できない場合

いずれかの task の verify step (`cargo check`) が dep resolve error で失敗した場合:

1. Error message から該当 crate 名と要求 version を特定。
2. `cargo search <crate>` で crates.io の最新 published version を確認。
3. root `[workspace.dependencies]` の該当 entry を利用可能 version に変更 (major.minor.patch を可能な範囲で blitz と揃える。blitz が pre-release を使っていて crates.io に無い場合は最新 stable に fallback)。
4. `bd remember` で version 変更の理由を記録 (spec §13 M0 の feasibility gate 検討材料)。
5. Task の Step を再実行。

version 変更が feasibility レベルの問題 (blitz 準拠不能、cssparser/selectors 版整合破綻) と判断できる場合は、nzv.3 を一旦 blocked にして nzv.10 (selectors-cssparser-version-compat) task 側で追跡する。

---

## Self-Review Checklist

**Spec coverage** (nzv.3 issue design section vs plan):
- 全 10 crate の Cargo.toml + src/lib.rs → Task 2–6 でカバー ✓
- root workspace.dependencies に spec §3 継承 dep + in-repo + dev-only oracle → Task 1 (external + dev-only) + Task 2–6 (in-repo 追加) ✓
- [package] の workspace 継承 + [lints] workspace = true → 全 crate に含む ✓
- cargo metadata + cargo check --workspace pass → Task 6 Step 8, 9 で最終 verify ✓
- Cargo.lock / version resolve verify → 明示的に nzv.4 に譲る (Global Constraints で明記) ✓
- Profile / virtual [package] block を入れない → Global Constraints で明記 ✓

**Placeholder scan:** "TBD" / "TODO (spec 外)" / "similar to Task N" のリファレンスなし。全 step で code block 提示済み。verify command と expected output 明示済み。

**Type consistency:** crate 名の書き方 (`raikiri-traits` vs `raikiri_traits`) が全体で `-` 形式に統一 (Cargo.toml / path 参照)。in-repo dep 参照は全て `{ workspace = true }`。version は全 crate `"0.1.0"`。

**Contingency:** version resolve failure に対する対応 flow を明記 (dep 側の feasibility は本 task の scope 外だが、実行中に遭遇し得る現実の失敗 mode)。
