# raikiri-spike-0hh: sub-error std::error::Error / Display impls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** raikiri-traits の残 sub-error 型 3 種 (`NetworkError` / `PolicyViolation` / `ResolverError`) に `impl std::fmt::Display` + `impl std::error::Error` を追加し、`RenderError::source()` を該当 variant で inner delegate 化して error-chain unwinding が premature に切れないようにする。

**Architecture:** m1.2 で確立済み pattern (`ParseError` / `CascadeError` / `LayoutError`) を Network/Policy/Resolver に adapt する。`match self` による variant 別 Display + `source()` (net) / `impl Error for T {}` default (uninhabited or leaf 型) の 2 form。`RenderError::source()` は match arm を追加して Resolver / Network / Policy 3 variant で `Some(e)` を返すよう更新。`NetworkError::PolicyViolation(PolicyViolation)` の入れ子は 2 段 chain として test で verify する。

**Tech Stack:** Rust (MSRV 1.89)、`std::error::Error` + `std::fmt::Display` (std 標準)、既存 dev-dep なし追加。`url` crate は raikiri-traits の通常 dep として既にあり test でも利用可。

## Global Constraints

- MSRV = 1.89 (§13.0 drift log 参照、`§M0` の `MSRV=1.85` 記述は歴史的記録)。
- raikiri-traits cleanroom 原則: 実装固有 error 型 (cssparser / html5ever / taffy 由来) を持ち込まない。今回追加する impl は std のみ依存。
- 全 sub-error 型に付与済みの `#[non_exhaustive]` 契約を維持。新 variant 追加や既存 field shape 変更は行わない。
- 設計仕様書 §4 の型 shape (NetworkError 5 variants / PolicyViolation 4 fields / ResolverError uninhabited-until-M4) は authoritative — spec 変更を伴う変形は本 plan の scope 外。
- Display 文言の style は m1.2 実装 (`"HTML parse error"` / `"CSS cascade internal error: {msg}"` / `"Layout internal error: {msg}"`) と同じく "<layer> <situation>" の短形。
- `#[non_exhaustive]` 型に対する `source()` match は残 variant を `Aborted | Http(_) | Other(_) => None` のように rest-cover し、将来追加 variant で compile break させる (spec 変更を要求する signal になる)。

---

## File Structure

**Modify only** — 新 file 無し。

- `crates/raikiri-traits/src/policy.rs` — `PolicyViolation` struct 定義直後に `impl Display` + `impl Error {}` を追加。
- `crates/raikiri-traits/src/net.rs` — `NetworkError` enum 定義直後に `impl Display` + `impl Error { source }` を追加。
- `crates/raikiri-traits/src/resolver.rs` — `ResolverError` enum 定義直後に `impl Display (match *self {})` + `impl Error {}` を追加。
- `crates/raikiri-traits/src/error.rs` — `RenderError::source()` 内の `Self::Resolver(_) | Self::Network(_) | Self::Policy(_)` を `Self::Resolver(e) => Some(e)` / `Self::Network(e) => Some(e)` / `Self::Policy(v) => Some(v)` に置換。
- `crates/raikiri-traits/src/lib.rs` — `── Sub-error trait bounds (M1.2) ──` セクション末尾に `── Sub-error trait bounds (0hh) ──` セクションを新設し、compile-time assertion 4 個 + runtime source_chain test 4 個 (計 8 test) を追加。

### 修正順序 (依存関係)

1. **PolicyViolation** — 最も leaf。他型の source() delegate 先。
2. **NetworkError** — `PolicyViolation` を source として `Some(&v)` 返すので PolicyViolation Error impl が先。
3. **ResolverError** — 独立。uninhabited enum。
4. **RenderError::source()** — Network / Policy / Resolver の Error impl 全部が揃った後に delegate arm 追加。
5. **Nested chain integration test** — `RenderError::Network(NetworkError::PolicyViolation(_))` の 2 段辿りを 1 test で verify。

---

## Task 1: PolicyViolation の Display + Error impl

**Files:**
- Modify: `crates/raikiri-traits/src/policy.rs` (add after line 86, i.e. after `PolicyViolation` struct definition, before `ViolationType` enum)
- Test: `crates/raikiri-traits/src/lib.rs` (add compile-time assertion in `mod tests`)

**Interfaces:**
- Consumes: 既存 `PolicyViolation` struct { kind: ResourceKind, url: Url, violation_type: ViolationType, details: String }。
- Produces: `impl std::fmt::Display for PolicyViolation` (uses violation_type / url / details in message) + `impl std::error::Error for PolicyViolation {}` (default `source()` returns None — struct に inner error field なし)。

- [ ] **Step 1: Write failing test**

`crates/raikiri-traits/src/lib.rs` の `── Sub-error trait bounds (M1.2) ──` セクションの末尾 (現在 `layout_error_is_error_and_display` の後、`render_error_parse_source_chain` の前が M1.2 sub-error group の境界。M1.2 group の直後、既存 M1.2 source_chain 系の前に新セクションを挿入する) に以下を追加:

```rust
    // ── Sub-error trait bounds (0hh) ────────────────────────────

    #[test]
    fn policy_violation_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<PolicyViolation>();
        _assert_display::<PolicyViolation>();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::policy_violation_is_error_and_display 2>&1 | tail -20`

Expected: **compile error** — `the trait bound 'PolicyViolation: std::error::Error' is not satisfied` (or `... Display ...`)。runtime assertion まで到達しない。

- [ ] **Step 3: Write minimal implementation**

`crates/raikiri-traits/src/policy.rs` の `PolicyViolation` struct 定義 (現在 L77-86) の直後 (L87 相当、空行を 1 行挟んで) に以下を追加:

```rust
impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Policy violation ({:?}) at {}: {}",
            self.violation_type, self.url, self.details
        )
    }
}

impl std::error::Error for PolicyViolation {}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::policy_violation_is_error_and_display 2>&1 | tail -10`

Expected: `test tests::policy_violation_is_error_and_display ... ok` (1 passed)。

- [ ] **Step 5: Verify no wider regression**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits 2>&1 | tail -5`

Expected: `test result: ok. 29 passed; 0 failed` (baseline 28 + 1 new)。

- [ ] **Step 6: Commit**

```bash
git add crates/raikiri-traits/src/policy.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): impl Display + Error for PolicyViolation (0hh)"
```

---

## Task 2: NetworkError の Display + Error impl

**Files:**
- Modify: `crates/raikiri-traits/src/net.rs` (add after line 158, i.e. after `NetworkError` enum definition — file 末尾)
- Test: `crates/raikiri-traits/src/lib.rs` (append 3 tests to `── Sub-error trait bounds (0hh) ──` section)

**Interfaces:**
- Consumes: 既存 `NetworkError` enum { Aborted, PolicyViolation(PolicyViolation), Io(std::io::Error), Http(u16), Other(String) } + Task 1 で追加した `impl Error for PolicyViolation`。
- Produces: `impl Display for NetworkError` (variant 別 message) + `impl Error for NetworkError { fn source() -> ... }` (PolicyViolation → Some(v)、Io → Some(e)、他 → None)。

- [ ] **Step 1: Write failing tests**

`crates/raikiri-traits/src/lib.rs` の `── Sub-error trait bounds (0hh) ──` セクション (Task 1 で新設済) の `policy_violation_is_error_and_display` test の直後に以下 3 test を追加:

```rust
    #[test]
    fn network_error_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<NetworkError>();
        _assert_display::<NetworkError>();
    }

    #[test]
    fn network_error_policy_source_chain() {
        use std::error::Error as _;
        use url::Url;
        let v = PolicyViolation {
            kind: ResourceKind::Image,
            url: Url::parse("https://example.com/x.png").unwrap(),
            violation_type: ViolationType::HostNotAllowed,
            details: String::from("host not in allowlist"),
        };
        let ne = NetworkError::PolicyViolation(v);
        let src = ne.source();
        assert!(
            src.is_some(),
            "NetworkError::PolicyViolation should expose inner PolicyViolation via source()"
        );
    }

    #[test]
    fn network_error_io_source_chain() {
        use std::error::Error as _;
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let ne = NetworkError::Io(io_err);
        let src = ne.source();
        assert!(
            src.is_some(),
            "NetworkError::Io should expose inner io::Error via source()"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::network_error 2>&1 | tail -20`

Expected: **compile error** — `the trait bound 'NetworkError: std::error::Error' is not satisfied` (or `... Display ...`)。3 test 全て compile 段階で fail。

- [ ] **Step 3: Write implementation**

`crates/raikiri-traits/src/net.rs` の `NetworkError` enum 定義 (現在 L145-158) の直後 (file 末尾、空行 1 行挟んで) に以下を追加:

```rust
impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted => write!(f, "Network fetch aborted"),
            Self::PolicyViolation(v) => {
                write!(f, "Network fetch violated policy: {:?}", v.violation_type)
            }
            Self::Io(_) => write!(f, "Network I/O error"),
            Self::Http(status) => write!(f, "Network HTTP status error: {status}"),
            Self::Other(msg) => write!(f, "Network error: {msg}"),
        }
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PolicyViolation(v) => Some(v),
            Self::Io(e) => Some(e),
            Self::Aborted | Self::Http(_) | Self::Other(_) => None,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::network_error 2>&1 | tail -10`

Expected: 3 test 全て `... ok` (total 3 passed)。

- [ ] **Step 5: Verify no wider regression**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits 2>&1 | tail -5`

Expected: `test result: ok. 32 passed; 0 failed` (baseline 28 + Task 1 1 + Task 2 3)。

- [ ] **Step 6: Commit**

```bash
git add crates/raikiri-traits/src/net.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): impl Display + Error for NetworkError (0hh)"
```

---

## Task 3: ResolverError の Display + Error impl (uninhabited enum)

**Files:**
- Modify: `crates/raikiri-traits/src/resolver.rs` (add after line 109, i.e. after `ResolverError` enum definition — file 末尾)
- Test: `crates/raikiri-traits/src/lib.rs` (append 1 test to `── Sub-error trait bounds (0hh) ──` section)

**Interfaces:**
- Consumes: 既存 `ResolverError` enum (M1.1 時点で variant 0 個、`#[non_exhaustive]`)。M4 で `Io / Decode / Timeout / NotSupported` variant が populate される予定。
- Produces: `impl Display for ResolverError { fn fmt() { match *self {} } }` (uninhabited 型に対する empty match、"never called at runtime" 契約) + `impl Error for ResolverError {}` (default source returns None、こちらも実行不能)。

- [ ] **Step 1: Write failing test**

`crates/raikiri-traits/src/lib.rs` の `── Sub-error trait bounds (0hh) ──` セクション末尾 (Task 2 の `network_error_io_source_chain` の直後) に以下を追加:

```rust
    #[test]
    fn resolver_error_is_error_and_display() {
        fn _assert_error<T: std::error::Error>() {}
        fn _assert_display<T: std::fmt::Display>() {}
        _assert_error::<ResolverError>();
        _assert_display::<ResolverError>();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::resolver_error_is_error_and_display 2>&1 | tail -10`

Expected: **compile error** — `the trait bound 'ResolverError: std::error::Error' is not satisfied` (or `... Display ...`)。

- [ ] **Step 3: Write implementation**

`crates/raikiri-traits/src/resolver.rs` の `ResolverError` enum 定義 (現在 L105-109) の直後 (file 末尾、空行 1 行挟んで) に以下を追加:

```rust
impl std::fmt::Display for ResolverError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for ResolverError {}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::resolver_error_is_error_and_display 2>&1 | tail -10`

Expected: `test tests::resolver_error_is_error_and_display ... ok` (1 passed)。

- [ ] **Step 5: Verify no wider regression**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits 2>&1 | tail -5`

Expected: `test result: ok. 33 passed; 0 failed` (baseline 28 + Task 1-2 4 + Task 3 1)。

- [ ] **Step 6: Commit**

```bash
git add crates/raikiri-traits/src/resolver.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): impl Display + Error for ResolverError (uninhabited, 0hh)"
```

---

## Task 4: RenderError::source() の Resolver / Network / Policy delegate

**Files:**
- Modify: `crates/raikiri-traits/src/error.rs` (replace body of `impl std::error::Error for RenderError` at L87-102)
- Test: `crates/raikiri-traits/src/lib.rs` (append 2 tests to `── Sub-error trait bounds (0hh) ──` section)

**Interfaces:**
- Consumes: Task 1-3 で追加した `impl Error for {PolicyViolation, NetworkError, ResolverError}`。
- Produces: `RenderError::source()` の match arm を更新し、`Self::Resolver(e) => Some(e)` / `Self::Network(e) => Some(e)` / `Self::Policy(v) => Some(v)` を追加。既存 delegate (Parse / Cascade / Layout / Sink / Io) は保持。`LimitExceeded / Configuration / TargetDidNotConverge` は inner error 不在のため `None` を維持。

- [ ] **Step 1: Write failing tests**

`crates/raikiri-traits/src/lib.rs` の `── Sub-error trait bounds (0hh) ──` セクション末尾 (Task 3 の `resolver_error_is_error_and_display` の直後) に以下 2 test を追加:

```rust
    #[test]
    fn render_error_network_source_chain() {
        use std::error::Error as _;
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let re = RenderError::Network(NetworkError::Io(io_err));
        let src = re.source();
        assert!(
            src.is_some(),
            "RenderError::Network should expose inner NetworkError via source()"
        );
    }

    #[test]
    fn render_error_policy_source_chain() {
        use std::error::Error as _;
        use url::Url;
        let v = PolicyViolation {
            kind: ResourceKind::ExternalStylesheet,
            url: Url::parse("https://cdn.example.com/main.css").unwrap(),
            violation_type: ViolationType::MimeNotAllowed {
                mime: String::from("text/plain"),
            },
            details: String::from("expected text/css"),
        };
        let re = RenderError::Policy(v);
        let src = re.source();
        assert!(
            src.is_some(),
            "RenderError::Policy should expose inner PolicyViolation via source()"
        );
    }
```

Resolver variant は `ResolverError` が uninhabited のため RenderError::Resolver(_) を construct できない → runtime test 不能。compile-time delegate は Task 4 の実装で cover される (source() match arm 内で `Self::Resolver(e) => Some(e)` 書けば `e: &ResolverError` が `&dyn Error` に coerce するかを compiler が確認)。

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::render_error_ 2>&1 | tail -20`

Expected: **既存 test は pass、新 2 test は fail** — assertion `src.is_some()` が false (現状 Network / Policy variant は `source()` で None を返す)。

- [ ] **Step 3: Write implementation**

`crates/raikiri-traits/src/error.rs` の `impl std::error::Error for RenderError` (現在 L87-102) を以下に置換:

```rust
impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            Self::Cascade(e) => Some(e),
            Self::Layout(e) => Some(e),
            Self::Resolver(e) => Some(e),
            Self::Network(e) => Some(e),
            Self::Policy(v) => Some(v),
            Self::Sink(e) | Self::Io(e) => Some(e),
            Self::LimitExceeded { .. }
            | Self::Configuration(_)
            | Self::TargetDidNotConverge { .. } => None,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::render_error_ 2>&1 | tail -10`

Expected: 既存 render_error_parse_source_chain + 新 2 test + render_error_is_error_trait 全て pass。

- [ ] **Step 5: Verify no wider regression**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits 2>&1 | tail -5`

Expected: `test result: ok. 35 passed; 0 failed` (baseline 28 + Task 1-3 5 + Task 4 2)。

- [ ] **Step 6: Commit**

```bash
git add crates/raikiri-traits/src/error.rs crates/raikiri-traits/src/lib.rs
git commit -m "feat(raikiri-traits): RenderError::source() delegate to Resolver/Network/Policy (0hh)"
```

---

## Task 5: Nested source chain integration test

**Files:**
- Test: `crates/raikiri-traits/src/lib.rs` (append 1 test to `── Sub-error trait bounds (0hh) ──` section)

**Interfaces:**
- Consumes: Task 1-4 全ての Error impl + RenderError::source() delegate。
- Produces: `RenderError::Network(NetworkError::PolicyViolation(_))` の source chain を 2 段辿り、末端 `PolicyViolation.source() == None` を確認する regression test。

**Purpose:** m1.2 で確立した "chain unwinding" が Consumer 側 (Network → Policy) の入れ子で end-to-end に効くことを 1 test で保証する。Task 4 単体 test は 1 段の delegate しか見ないため、2 段辿りは別 test で明示する。

- [ ] **Step 1: Write test**

`crates/raikiri-traits/src/lib.rs` の `── Sub-error trait bounds (0hh) ──` セクション末尾 (Task 4 の `render_error_policy_source_chain` の直後) に以下を追加:

```rust
    #[test]
    fn render_error_network_policy_nested_source_chain() {
        use std::error::Error as _;
        use url::Url;
        let v = PolicyViolation {
            kind: ResourceKind::Image,
            url: Url::parse("http://tracker.example.com/1x1.gif").unwrap(),
            violation_type: ViolationType::SchemeNotAllowed,
            details: String::from("http not allowed in strict mode"),
        };
        let re = RenderError::Network(NetworkError::PolicyViolation(v));
        // depth 1: RenderError → NetworkError
        let inner = re
            .source()
            .expect("RenderError::Network should delegate to NetworkError");
        // depth 2: NetworkError::PolicyViolation → PolicyViolation
        let deep = inner
            .source()
            .expect("NetworkError::PolicyViolation should delegate to PolicyViolation");
        // depth 3: PolicyViolation is a leaf (no inner error)
        assert!(
            deep.source().is_none(),
            "PolicyViolation should be the leaf of the chain"
        );
    }
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits tests::render_error_network_policy_nested_source_chain 2>&1 | tail -10`

Expected: `test tests::render_error_network_policy_nested_source_chain ... ok` (1 passed)。

- [ ] **Step 3: Verify final test count**

Run: `cargo test --manifest-path Cargo.toml --package raikiri-traits 2>&1 | tail -5`

Expected: `test result: ok. 36 passed; 0 failed` (baseline 28 + 8 new tests: 4 assertion + 4 source_chain — Task 1 の 1 + Task 2 の 3 + Task 3 の 1 + Task 4 の 2 + Task 5 の 1)。

- [ ] **Step 4: Cross-workspace verify**

Run: `cargo test --manifest-path Cargo.toml --workspace 2>&1 | tail -20`

Expected: 全 workspace test pass (raikiri-traits 依存側 — raikiri, raikiri-html, raikiri-dom, raikiri-style, raikiri-paint, raikiri-net — で `impl Error for {NetworkError, PolicyViolation, ResolverError}` を前提とした既存 code は無いはずで regression しない)。

- [ ] **Step 5: Commit**

```bash
git add crates/raikiri-traits/src/lib.rs
git commit -m "test(raikiri-traits): nested source chain RenderError → NetworkError → PolicyViolation (0hh)"
```

---

## Self-Review

**1. Spec coverage:**

Description の 4 sub-scope:
- (a) NetworkError に `impl Error + Display` → **Task 2**
- (b) PolicyViolation に `impl Error + Display` → **Task 1**
- (c) ResolverError に `impl Error + Display` → **Task 3**
- (d) RenderError::source() の match arm 更新 (Resolver / Network / Policy) → **Task 4**

追加で Task 5 (nested chain integration test) を入れて m1.2 pattern を延長。

**2. Placeholder scan:** 全 step に concrete code block / concrete command / concrete expected output が入っている。TBD / TODO / "similar to Task N" 無し。

**3. Type consistency:**
- `PolicyViolation` field 名 (kind / url / violation_type / details) — Task 1 実装、Task 2 test、Task 4 test で一致。
- `NetworkError` variant 名 (Aborted / PolicyViolation / Io / Http / Other) — Task 2 実装 + Task 4 test で一致。
- `ResolverError` — Task 3 で uninhabited 前提を明示、Task 4 で `Self::Resolver(e) => Some(e)` の delegate はあるが runtime test 無しを明示。
- `ResourceKind` (`Image` / `ExternalStylesheet`) と `ViolationType` (`HostNotAllowed` / `MimeNotAllowed { mime }` / `SchemeNotAllowed`) は policy.rs 現行 variant を利用 (追加 variant なし)。
- `Url::parse` — `url` crate は raikiri-traits の通常 dep として既にある (workspace-inherited)。

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-16-raikiri-spike-0hh-sub-error-impls.md`.
