# scripts/wpt/

`fetch.sh` executes a sparse, shallow clone of the W3C web-platform-tests
repository into `target/wpt/`, pinned to the SHA in `pinned_sha.txt`.

The set of fetched paths is controlled by `subset.txt` (one pattern per line,
Git sparse-checkout syntax). Currently M1 scope: `fonts` only (Ahem, Lato,
CSSTest 等) for VRT cross-machine determinism (raikiri-spike-e93).

## Usage

    scripts/wpt/fetch.sh

Idempotent: re-running updates to the current pinned SHA. Override the remote
URL with `WPT_REMOTE_URL=...` (mirrors / CI cache warmup).

## Updating the pin

The pin is initially borrowed from fulgur (`fulgur/scripts/wpt/pinned_sha.txt`)
for cross-project consistency. Bump raikiri's pin only when:

1. fulgur bumps and raikiri should follow (default), OR
2. raikiri 専有 regression requires a fresh WPT font asset (rare)

Steps:

1. Inspect upstream WPT `main` (or fulgur's next pin) and pick a green commit
2. Replace the SHA line in `pinned_sha.txt`
3. Re-run `scripts/wpt/fetch.sh`
4. Re-run `cargo test -p raikiri --test hello_world_vrt -- --ignored`
   (VRT test is `#[ignore]` by default — see hello_world_vrt.rs). Golden PNG
   may need regeneration if font asset content shifted:
   `RAIKIRI_UPDATE_GOLDENS=1 cargo test -p raikiri --test hello_world_vrt -- --ignored`
5. Commit `pinned_sha.txt` + updated golden (if any) in one PR

## Relation to `raikiri-wpt`

`raikiri-wpt` (WPT test runner harness) does not currently depend on
`target/wpt/`. This subset is dedicated to raikiri VRT font pin
(raikiri-spike-e93). When `raikiri-wpt` starts consuming WPT test
resources, extend `subset.txt` and update this README.

## Updating the CSS dashboard

`docs/wpt-dashboard.html` is generated from the pinned WPT tree and
`expectations/raikiri-baseline.txt`; do not edit its counters by hand. From a
working tree, run:

    scripts/wpt/update-dashboard.sh

The wrapper fetches the pinned WPT checkout, runs the `raikiri-wpt` smoke
reftests, records their passed count, and invokes `generate-dashboard`. The
page reports that smoke count alongside the T2 baseline coverage. The
generator reads the WPT Git tree rather than only the sparse working files, so
the CSS denominator remains complete. To regenerate without rerunning the
smoke tests, invoke the binary directly (the optional smoke count can be
supplied when it is available):

    cargo run --locked -p raikiri-wpt --bin generate-dashboard -- \
        --wpt-root target/wpt \
        --baseline expectations/raikiri-baseline.txt \
        --basic-reftest-count 12 \
        --output docs/wpt-dashboard.html
