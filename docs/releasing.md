# Versioning before crates.io publication

Every member of the main Cargo workspace inherits `[workspace.package].version`.
The development series starts at **0.0.1** and advances in patch increments,
including for API breaking changes. These versions do not promise API stability.
The separate Wasmtime tools workspace keeps its own package versions; its Raikiri
path dependencies follow the main workspace version.

## Release flow

1. A push to `main` starts the **Release-plz** workflow. It can also be started
   manually with **Run workflow**, selecting `main`; leave **release** off to
   only update the Release PR.
2. On the first run, with version `0.0.1` and no `raikiri-v*` tag, the workflow
   tags the initial version. No version bump PR is needed for this baseline.
3. Later pushes create or update a Release PR that changes the shared version,
   internal dependency requirements, and lockfiles. The Wasmtime tools lockfile
   is synchronized on the same PR using the existing GitHub App token.
4. Review and merge that PR. The workflow run of the merge detects the Release
   PR (its `release-plz-<date>` branch in the merge subject, or a
   `chore: release vX.Y.Z` commit in the push) and starts the **release** job,
   which waits for an approval of the `crates-io` environment.
5. After the approval, the job publishes the eight crates listed under
   [Which crates are published](#which-crates-are-published) to crates.io and
   tags each workspace member with `<package>-v<version>`, for example
   `raikiri-v0.0.6`. Rejecting the approval publishes and tags nothing.

All 14 main workspace members share the version, including private test and
benchmark crates. New members must also use `version.workspace = true`.
The workflow only runs its release jobs on `main`. Tag creation and publishing
complete before version analysis in each run: the Release PR job waits for the
release job, and runs when the release job is skipped because the push is not a
release. The release job is serialized so that two runs cannot publish the same
version; an ordinary push skips it and never waits for an approval.

`release-plz.toml` keeps git-only mode and disables Cargo publishing for the
workspace, and turns both around for the eight published crates: release-plz
reads their latest version from crates.io and publishes them, and reads the
others from their git tags and only tags them. GitHub Releases stay disabled.
The Release PR uses the existing `RELEASE_PLZ_APP_CLIENT_ID` repository variable
and `RELEASE_PLZ_APP_PRIVATE_KEY` secret so PR CI can run. Tag creation uses
`GITHUB_TOKEN`. Publishing uses crates.io Trusted Publishing: the release job
runs in the `crates-io` environment with `id-token: write`, and
`rust-lang/crates-io-auth-action` exchanges the GitHub OIDC token for a
short-lived crates.io token. Each published crate trusts the workflow
`release-plz.yml` of `fulgur-rs/raikiri` in the environment `crates-io` on
crates.io; no long-lived token is stored. Both release-plz jobs pin release-plz
to the locally verified **0.4.0**.

The initial-release configuration temporarily sets `release_always = true`.
After the baseline exists, the regular `release_always = false` setting requires
a merged Release PR for tagging. Automatic PR merging is not enabled.

## Local verification

```bash
cargo metadata --no-deps --locked --format-version 1
cargo metadata --no-deps --locked --format-version 1 --manifest-path tools/raikiri-js-wasmtime/Cargo.toml
scripts/publish-dry-run.sh --allow-dirty   # packages the crates for crates.io
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

## Publishing to crates.io

The eight crates below are published by the release job of the Release-plz
workflow (see [Release flow](#release-flow)). Before their first publication
each name existed on crates.io only as a `0.0.0` reservation.

### Which crates are published

Eight crates carry the public API (see [public-api.md](public-api.md)). In
dependency order:

1. `raikiri-style` and `raikiri-layout-geometry`, which depend on no other
   workspace crate
2. `raikiri-traits` and `raikiri-layout-style`
3. `raikiri-svg`
4. `raikiri-net`
5. `raikiri-dom`
6. `raikiri-html`

`raikiri` (dogfooding), `raikiri-paint` and `raikiri-js` carry `publish = false`,
like the test, benchmark and diff tools. A crate that should leave that list
needs its own decision, its API review and an entry in `scripts/publish-dry-run.sh`.
The script fails when the manifests and its list disagree.

### Rules for the manifests

- Dependencies on other workspace crates are declared once in the root
  `[workspace.dependencies]` with both `version` and `path`; cargo publishes the
  version and drops the path.
- A dev-dependency on a sibling that depends back on the crate must be
  declared with a path and **no version**. Cargo removes it from the published
  manifest; with a version it would stay in, and the crates could not be
  published in any order. `raikiri-dom` does this for `raikiri-html`. The
  consequence is that the tests of a downloaded `raikiri-dom` package that use
  `raikiri-html` cannot be built from the package alone.
- No published crate may depend on code that is not on crates.io: no `[patch]`,
  no git dependency, no path dependency (the version-less dev-dependency above
  is the one exception, because cargo drops it), no vendored or forked copy.
- Each published crate has a `description`, a `readme`, `keywords` and
  `categories` (at most five keywords and five categories, which crates.io
  enforces).

### Checking without credentials

```bash
scripts/publish-dry-run.sh                # CI runs this on every pull request
scripts/publish-dry-run.sh --allow-dirty  # with uncommitted changes
```

It runs `cargo publish --dry-run` for all eight crates at once, with the
`http-ureq` feature of `raikiri-net` on. Cargo packages them in dependency order
and resolves a dependency on another one from its package, so the check works
while the new versions are not on crates.io. It needs no token and uploads
nothing. Beyond the manifest metadata it catches a package that does not build on
its own, for example one that reads a file outside its crate directory, and a
dependency cycle that publishing would hit. It builds the library of each package
and not its tests.

### License

The repository root holds `LICENSE-MIT` and `LICENSE-APACHE` (copyright Lutom
LLC. and contributors), matching `license = "MIT OR Apache-2.0"`. Each published
crate links both files into its directory, so `cargo package` puts the text in
every package.

### First publication

release-plz treats a package whose version already has a git tag as released,
so it never publishes `0.0.5`: that version was tagged while the crates were
git-only. Enabling publishing therefore comes with the shared version moved to
`0.0.6`, which has no tag. In that state `release-pr` proposes no further bump,
and nothing is published until someone asks for it:

1. Start **Release-plz** with **Run workflow** on `main` and **release** on.
2. Approve the `crates-io` environment of the release job.
3. The job publishes the eight crates at `0.0.6` in dependency order and tags
   every workspace member `<package>-v0.0.6`.

The same manual run releases the current version whenever a merged Release PR
was not detected; packages already on crates.io or already tagged are skipped.
From `0.0.6` on, the registry and the tags agree, and later versions go through
Release PRs as described in [Release flow](#release-flow).

### Notes

- **Package size.** `raikiri-style` and `raikiri-dom` are the largest packages
  (1.4 MiB and 1.6 MiB compressed, mostly sources and the unit tests next to
  them). crates.io accepts up to 10 MiB, so no change is needed.

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
