# Versioning before crates.io publication

Every member of the main Cargo workspace inherits `[workspace.package].version`.
The development series starts at **0.0.1** and advances in patch increments,
including for API breaking changes. These versions do not promise API stability.
The separate Wasmtime tools workspace keeps its own package versions; its Raikiri
path dependencies follow the main workspace version.

## Release flow

1. A push to `main` starts the **Release-plz** workflow. It can also be started
   manually with **Run workflow**, selecting `main`.
2. On the first run, with version `0.0.1` and no `raikiri-v*` tag, the workflow
   tags the initial version. No version bump PR is needed for this baseline.
3. Later pushes create or update a Release PR that changes the shared version,
   internal dependency requirements, and lockfiles. The Wasmtime tools lockfile
   is synchronized on the same PR using the existing GitHub App token.
4. Review and merge that PR. The next workflow run tags each workspace member
   with `<package>-v<version>`, for example `raikiri-v0.0.2`.

All 14 main workspace members share the version, including private test and
benchmark crates. New members must also use `version.workspace = true`.
The workflow only runs its release jobs on `main`. Tag creation completes
before version analysis in each run. Only Release PR jobs use concurrency
control: tag jobs must run for every merge, including when newer pushes arrive.

`release-plz.toml` enables git-only mode and explicitly disables Cargo publishing
and GitHub Releases. Neither a crates.io token nor `cargo publish` is used.
The Release PR uses the existing `RELEASE_PLZ_APP_CLIENT_ID` repository variable
and `RELEASE_PLZ_APP_PRIVATE_KEY` secret so PR CI can run. Tag creation uses
`GITHUB_TOKEN`. Both jobs pin release-plz to the locally verified **0.4.0**.

The initial-release configuration temporarily sets `release_always = true`.
After the baseline exists, the regular `release_always = false` setting requires
a merged Release PR for tagging. Automatic PR merging is not enabled.

## Local verification

```bash
cargo metadata --no-deps --locked --format-version 1
cargo metadata --no-deps --locked --format-version 1 --manifest-path tools/raikiri-js-wasmtime/Cargo.toml
```

For release-plz upgrades, use a dependency-free temporary workspace with two
libraries and one `publish = false` helper, all inheriting version `0.0.1`.
Make the second library depend on the first with a workspace path dependency
and version `0.0.1`. Use this repository's release-plz configuration and run
`release-plz update --repo-url https://github.com/fulgur-rs/raikiri` locally:

- Without baseline tags: the version stays `0.0.1` (the workflow bootstraps tags).
- With `<package>-v0.0.1` tags and no new changes: no version change.
- After a `fix:`, `feat:`, or `feat!:` commit in one library: all three package
  versions and the internal dependency requirement become `0.0.2`.
- A change only to the private helper also advances all three to `0.0.2`.

Verify these cases without invoking `release-pr` or pushing fixture tags to GitHub.
The same cases, plus changing only the second library and rerunning an update,
passed with release-plz 0.4.0 during this migration. The workflow shell was also
checked with fixtures: initial/no-tags enables baseline tagging, an existing
baseline or a later version keeps normal Release PR gating, and tools lockfile
synchronization commits/pushes to a local bare remote only when the file changes.

Actions syntax was checked with actionlint 1.7.12. Its bundled definition of
`actions/create-github-app-token@v3` predates the `client-id` input, so only its
missing-`app-id` and unknown-`client-id` diagnostics were excluded after checking
the official v3 action definition; the existing `client-id` configuration is valid.

Refreshing the tools lockfile also reconciled pre-existing manifest drift:
`shodo` was still locked to `0.0.32` despite the shared manifest pin of `0.0.37`,
and the lockfile was missing the layout-style and color-management dependencies.
The main workspace lockfile changes only local package versions.

## Removed pre-migration tags

The following old Git tags are deleted as requested when resetting the version
series. The SHAs are retained here so their previous targets remain identifiable.
These were Git tags, not crates.io publications.

| Tag | Previous target SHA |
| --- | --- |
| `raikiri-blitz-compat-v0.1.0` | `f3189840c75c7ef8711721e0d7e33edef27cf850` |
| `raikiri-dom-v0.1.0` | `a4755fcb69eb19939653e676f8fb52d80e0234c2` |
| `raikiri-html-v0.1.0` | `811bb2b7a5e3952f47d39c80643db6193bf3fd63` |
| `raikiri-js-v0.1.0` | `86bc1c484b5aa3f84b7144a1194b24e040fafb91` |
| `raikiri-net-v0.1.0` | `89b0c2d97f46cad22c88d274bd03052c1d1f2448` |
| `raikiri-paint-v0.1.0` | `4fb87502abd6784c4bb91ba6c945ba4701985610` |
| `raikiri-style-v0.1.0` | `f4301e594b1120213dcd971b18f6d659cf5e39f9` |
| `raikiri-traits-v0.2.0` | `793755cb8b9696feed14f87c79dbe0b24be5603b` |
| `raikiri-v0.1.0` | `bbea716a78f173cc1790ee9e52983437a77caf64` |

References: [release-plz configuration](https://release-plz.dev/docs/config),
[shared workspace versions](https://release-plz.dev/docs/usage/set-version),
and [0.0.x increment behavior](https://docs.rs/next_version/latest/src/next_version/lib.rs.html).
