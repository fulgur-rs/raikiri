# Vendored Parley: provenance and upgrade check

Raikiri patches Parley to carry CSS `line-break:anywhere` break opportunities into layout while preserving mandatory newlines and CSS whitespace behavior. Keep this maintenance guide separate from runtime behavior changes.

## Upstream source

- Crate: `parley` 0.11.1.
- Repository: <https://github.com/linebender/parley>.
- Upstream source revision: `eea3503dd6cf17130cbb07348e0ff2c918300e94` (`vendor/parley/.cargo_vcs_info.json`).
- The workspace applies the local copy through `[patch.crates-io]` in the root `Cargo.toml`.

Before changing the vendored version, record the new crate version and upstream revision in both the root patch comment and this file. Compare the replacement with the upstream package before reapplying local edits. The current local Rust diff against the 0.11.1 registry package is limited to these five files:

| File | Local purpose |
| --- | --- |
| `vendor/parley/src/builder.rs` | Add ranged-builder flags and setters for `line-break:anywhere`, `white-space:break-spaces`, and `white-space:pre-wrap` hanging spaces; pass those flags into resolved layout styles while leaving ordinary builder paths disabled. |
| `vendor/parley/src/context.rs` | Initialize the new builder flags to `false`. |
| `vendor/parley/src/layout/mod.rs` | Store the three resolved per-style flags. |
| `vendor/parley/src/resolve/mod.rs` | Initialize the flags to `false` for ordinary resolved styles. |
| `vendor/parley/src/layout/line_break.rs` | Process mandatory newlines before soft opportunities; let `anywhere` break before `break-spaces` whitespace and NBSP rather than hanging it; preserve `pre-wrap` trailing-space hanging without inserting a soft break before a following mandatory newline. |

## Repeatable upgrade check

Run this sequence after replacing or rebasing the vendored Parley source:

```sh
scripts/wpt/fetch.sh
cargo tree --locked -p raikiri-dom -i parley
cargo test --locked -p parley
cargo test --locked -p raikiri-dom -p raikiri-paint
cargo test --locked -p raikiri-wpt --test css_line_break_reftests
cargo test --locked -p raikiri-wpt --test css_line_break_reftests -- --ignored --test-threads=1
cargo fmt --all --check
cargo clippy --locked -p raikiri-dom -p raikiri-paint -p raikiri-wpt --all-targets -- -D warnings
```

Check that `cargo tree` resolves Parley through this workspace's `vendor/parley` path. Parley's upstream unit tests run with `cargo test -p parley`; they cover Parley boundary overrides and mandatory breaks. The DOM/paint tests cover the local layout/paint integration. The first WPT command runs the synthetic whitespace/newline regressions. The ignored-WPT command runs the pinned 24-case first-stage suite and the four shared-inline cases at exact 800×600. All must pass against their real references.

## Raikiri integration points

`crates/raikiri-dom/src/layout/inline_text.rs` applies the Parley override and style flags in three builder routes: shared-inline shaping, serial text shaping, and parallel text shaping. Keep all three calls in sync with any Parley API change. The `Cargo.toml` path patch and the path entry in `Cargo.lock` must also continue to resolve to `vendor/parley`.

Also review the diff against the new upstream package and confirm that the five vendor patch points above and all three Raikiri activation points remain present, or that equivalent behavior is covered by tests. Do not update `expectations/raikiri-baseline.txt` to hide an upgrade regression. Any baseline movement or change to CSS behavior requires its own scoped review.

## Known limit

The `pre-wrap` hanging-space scan currently walks clusters in the current Parley text run. A preserved trailing-space sequence that crosses a Parley style/run boundary can still spill to the next line. This is tracked with the shared-inline formatting work; do not silently broaden this vendor patch during an unrelated Parley bump.
