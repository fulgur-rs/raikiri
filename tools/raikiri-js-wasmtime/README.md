# Local Wasmtime WPT trial

This independent workspace builds the real raikiri-js DomRuntime as a WASI
reactor. Logical DOM snapshots call the existing native WPT CSS/layout host.
Backend selection is compile-time only: native is the WPT default; use
`--no-default-features --features js-wasmtime` for the trial.

Build explicitly with `bash tools/raikiri-js-wasmtime/scripts/build.sh`.
The trusted AOT compiler and runtime use Wasmtime 43.0.2 and identical fuel,
512KiB stack, 128MiB reservation and 64KiB guard settings. The script writes
artifact hashes/configuration beside the AOT file and embeds it with the
`RAIKIRI_JS_AOT` compile-time environment variable. There are no Cargo or compiler
calls from build.rs. Guest JSON messages are capped at 16MiB.

The runtime executable contains AOT code and does not need a guest/artifact file
at runtime. Zero-copy executable publishing is restricted to Linux x86_64 with
4KiB pages. The process deserializes exactly one Module and shares it across
independent Stores. Do not independently deserialize overlapping embedded bytes.
The custom publisher never grants write permission. This is a local trial,
not a production sandbox or a macOS/Windows support claim.

Per-document defaults: 10 billion fuel units and 128MiB linear memory. WPT trial
options `RAIKIRI_WPT_FUEL` and `RAIKIRI_WPT_MEMORY_BYTES` change those budgets;
`RAIKIRI_WPT_METRICS=1` prints bridge/fuel/memory data. The budget includes realm
initialization, parsing, callbacks, result getters and teardown, without refills.
Native CSS/layout/file/fragment work is outside these limits, so retain an outer
process timeout. Minimal WASI exposes deterministic synthetic randomness/time,
empty environment and discarded writes; no filesystem/network capability.

Both suite runners accept `--results-json PATH`. `scripts/compare.py` compares
file errors and ordered named assertion statuses, retaining duplicate names and
reporting message-only differences separately. Existing WPT expectations remain
unchanged. `scripts/measure.py REPO_ROOT` runs freshly copied release binaries
under `target/integration-logs/{native,wasmtime}-bins` sequentially, samples RSS
and records hashes/timing. Stop other builds/tests during timing runs.

A clean guest Abort preserves final logical mutations when the final transfer succeeds.
A later result or DOM transfer failure preserves the known Abort as the page outcome
and logs the transfer failure separately; only the last native checkpoint remains.
An engine trap discards
the Store and preserves only the latest synchronized native checkpoint; the host
never calls exports on the stopped Store. Fonts remain native and separate.

See [RESULTS.md](RESULTS.md) for complete WPT parity, measured overhead,
artifact hashes and the retained trial limitations.
