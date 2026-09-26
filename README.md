# raikiri

[![CI](https://github.com/fulgur-rs/raikiri/actions/workflows/ci.yml/badge.svg)](https://github.com/fulgur-rs/raikiri/actions/workflows/ci.yml)

A Rust-based HTML/CSS paged layout and rendering foundation developed to replace fulgur's layout engine.
It aims to provide an incremental migration path from fulgur. It parses HTML, applies CSS cascading and layout, and currently focuses on single-page PNG output.
Compatibility is validated with paged-media CSS and W3C Web Platform Tests (WPT) as development continues toward fulgur's successor and replacement engine.
Unlike general-purpose layout engines, raikiri treats pagination and typesetting-oriented architecture as first-class concepts.

> **Status:** Under active development. The API and implementation scope may change.
> `layout()` returns retained pages and fragments for consumers to inspect and draw.

## For users

### Goal

- Incrementally replace fulgur's layout engine with an implementation centered on pagination and typesetting
- Treat pagination, page contexts, and typesetting as first-class rendering models
- Implement the HTML/CSS features needed for document rendering and validate compatibility with WPT
- Provide APIs and rendering paths that let existing users migrate incrementally
- Keep document processing and layout extensible for future JavaScript support
- Target AI-agent workloads while balancing cold-start performance, low memory footprint, and security

### Non-goal

- Replacing a general-purpose browser, including JavaScript, all at once in the short term
- Implementing every HTML/CSS specification at once
- Becoming a general-purpose UI layout engine that does not assume pagination or typesetting

### What it supports

- HTML parsing and DOM construction with html5ever
- CSS cascading with `@page`, the user-agent stylesheet, and inline stylesheets
- Type, universal, class, ID, and attribute selectors, plus basic combinator matching
- Block / flex / grid layout, text shaping, and page-fragment painting
- Single-page PNG output with `raikiri::html_to_png`
- PNG image fetching, decoding, and painting with `raikiri::html_to_png_with_resolver`
- VRT (visual regression testing) and WPT reftest infrastructure

### Minimal example

Within the repository, the `raikiri` umbrella crate provides a single entry point for HTML parsing and CSS cascading.

```rust
use raikiri::{build_cascaded, parse, ParseOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = parse(&b"<p style=\"color: red\">Hello, raikiri!</p>"[..], &options)?;
    let cascade = build_cascaded(&document);

    assert!(!cascade.computed.is_empty());
    Ok(())
}
```

Use the following API to generate a PNG:

```rust
use std::io::Cursor;
use raikiri::html_to_png;

let png = html_to_png(Cursor::new(b"<p>Hello, raikiri!</p>"))?;
std::fs::write("output.png", png)?;
# Ok::<(), raikiri::RenderError>(())
```

`html_to_png` renders the first page only. It uses A4 when no `@page` size is specified.
Use `html_to_png_with_fonts` when reproducible fonts are required for VRT.

## For developers

### Crate layout

| Crate | Role |
| --- | --- |
| [`raikiri`](crates/raikiri) | User-facing facade. Re-exports parsing, cascading, PNG output, and core types |
| [`raikiri-traits`](crates/raikiri-traits) | Shared traits and types for the DOM, network, resolver, rendering, and page models |
| [`raikiri-html`](crates/raikiri-html) | html5ever wrapper and HTML → uncascaded document pipeline |
| [`raikiri-style`](crates/raikiri-style) | cssparser / selectors integration, rule tree, cascade, and computed values |
| [`raikiri-dom`](crates/raikiri-dom) | DOM arena, taffy layout, parley text processing, and page state |
| [`raikiri-net`](crates/raikiri-net) | `NetworkProvider`, image resolver, and PNG decoding |
| [`raikiri-paint`](crates/raikiri-paint) | Builds anyrender paint scenes from page fragments |
| [`raikiri-vrt`](crates/raikiri-vrt) | Fixtures and diff infrastructure for visual regression tests (private crate) |
| [`raikiri-wpt`](crates/raikiri-wpt) | WPT runner, reftests, blitz oracle, and expectations lint (private crate) |

The main pipeline is:

```text
HTML bytes
   │
   ▼
raikiri-html ──► raikiri-dom
                      │
CSS / UA stylesheet ──┴──► raikiri-style (cascade)
                                   │
                                   ▼
                         raikiri-dom (layout)
                                   │
                                   ▼
                         raikiri-paint (scene)
                                   │
                                   ▼
                         anyrender / PNG
```

### Setup

Requirements:

- Rust **1.91.0** (pinned by `rust-toolchain.toml`; `mise.toml` pins the same version for `mise` users)
- rustup
- On Linux, `libfontconfig1-dev` and `pkg-config`

```bash
rustup show active-toolchain
cargo build --workspace --locked
```

Install the `rustfmt` and `clippy` components specified by `rust-toolchain.toml` with rustup.
If font-related dependencies fail to build, install the Debian/Ubuntu packages:

```bash
sudo apt-get update
sudo apt-get install -y libfontconfig1-dev pkg-config
```

### Tests and quality checks

Run the workspace tests:

```bash
cargo test --workspace --locked
```

Run the same format and clippy checks as CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Run the project gate as one command:

```bash
scripts/gate.sh --skip-coverage
```

`--skip-coverage` is intended for the fast development loop. Before submitting changes, run the required gate and patch coverage with `cargo-llvm-cov` available.

### VRT and WPT

#### VRT

The ignored hello-world VRT uses pinned WPT fonts, such as Ahem, to keep output reproducible across machines.
Fetch the required WPT subset once before running it:

```bash
scripts/wpt/fetch.sh
cargo test -p raikiri --test hello_world_vrt -- --ignored
```

Update goldens only when intentionally changing rendering behavior:

```bash
RAIKIRI_UPDATE_GOLDENS=1 \
  cargo test -p raikiri --test hello_world_vrt -- --ignored
```

See [`crates/raikiri/tests/reference/hello-world/README.md`](crates/raikiri/tests/reference/hello-world/README.md) for the fixture details.

#### WPT expectations and dashboard

Validate the expectations files:

```bash
cargo run --locked -p raikiri-wpt --bin validate-expectations
```

Run the WPT smoke reftests and regenerate the dashboard:

```bash
scripts/wpt/update-dashboard.sh
```

The generated page is [`docs/wpt-dashboard.html`](docs/wpt-dashboard.html). See [`scripts/wpt/README.md`](scripts/wpt/README.md) for WPT fetching and pin-update procedures.

### Documentation

- [M0 feasibility report](docs/feasibility-report.md)
- [WPT dashboard](docs/wpt-dashboard.html)
- [WPT workflow](scripts/wpt/README.md)
- [hello-world VRT fixture](crates/raikiri/tests/reference/hello-world/README.md)

Design decisions and work status are recorded in the repository's Beads issue tracker (`bd`).

### License

Dual-licensed under the MIT License or Apache License 2.0.
