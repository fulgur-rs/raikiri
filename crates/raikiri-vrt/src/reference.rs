//! raikiri-vrt::reference — reference-fixture harness for VRT tests.
//!
//! Loads `tests/reference/<name>/` fixtures, invokes a caller-supplied
//! rendering pipeline, and compares the produced PNG bytes against
//! `expected/page-{N:04}.png` under a caller-selected pixel tolerance.
//!
//! On failure, writes `target/reference-diffs/<name>/page-{N:04}-{actual,diff}.png`
//! for post-mortem inspection.
//!
//! Golden-update mode: setting `RAIKIRI_UPDATE_GOLDENS=1` in the environment
//! recreates `expected/` from the pipeline output instead of comparing.

// d9y.6 threat-model docs cross-link `FIXTURE_SIZE_CAP` / `MAX_EXPECTED_PAGES`
// / `FIXTURE_AGGREGATE_BYTES_CAP` / `read_bounded_fixture_file` (module-private
// items) from public `FixtureError` variants and `load_fixture` docstring, so
// the links resolve under `--document-private-items` but trip `-D warnings` on
// the public build. `publish = false` dev-only crate; keep the cross-links.
#![allow(rustdoc::private_intra_doc_links)]

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Open a regular file with the leaf-swap TOCTOU defense stack.
///
/// The unix impl passes `O_NOFOLLOW | O_NONBLOCK` to `File::open`:
///
/// - `O_NOFOLLOW`: a symlink swapped in between the pre-open
///   `symlink_metadata` check and this open call cannot cause the resolver
///   to follow a fresh target. POSIX mandates `ELOOP` for
///   `open(O_NOFOLLOW)` on a symlink (Linux, macOS, and modern FreeBSD
///   comply); legacy BSDs (NetBSD, OpenBSD, FreeBSD <10) may return
///   `EMLINK` or `EFTYPE` instead. Callers that need to distinguish this
///   case from other I/O errors should check the errno match against
///   `libc::ELOOP` AND fall back to a `symlink_metadata` recheck (see
///   `read_bounded_fixture_file` for the portable pattern).
/// - `O_NONBLOCK`: **load-bearing time-DoS defense** paired with the
///   post-open fstat check in [`check_open_handle_regular`]. If a regular
///   file is swapped for a **writer-less FIFO** between the pre-open
///   `symlink_metadata` check and this `open()`, the bare
///   `open(O_RDONLY | O_NOFOLLOW)` call would block indefinitely at the
///   `open()` syscall itself — `O_NOFOLLOW` does not fire (a FIFO is not a
///   symlink), so the post-open fstat is never reached. `O_NONBLOCK` makes
///   FIFO opens return immediately (POSIX: read-side `O_RDONLY | O_NONBLOCK`
///   on a FIFO succeeds even with no writer), letting the post-open fstat
///   inspect the fd and reject non-regular kinds. Regular file semantics
///   are unaffected: POSIX specifies `O_NONBLOCK` has no effect on regular
///   files, and Linux/macOS both honor that. bd raikiri-spike-z719 (ports
///   `raikiri_dom::fonts::safe_open`'s O_NONBLOCK, added there by bd
///   raikiri-spike-f4j — 61l Codex §8.3 finding #2).
///
/// The non-unix fallback keeps the current default `File::open` semantics.
/// Windows equivalent tracked in raikiri-spike-akk.
///
/// bd raikiri-spike-8yu (O_NOFOLLOW), raikiri-spike-z719 (O_NONBLOCK).
// Callsite-local defense: sharing this stack with raikiri-dom is deferred to
// raikiri-spike-7xw for walls.md §2 crate-list PMO judgment. Do not lift
// into raikiri-traits::io without that judgment.
#[cfg(unix)]
fn safe_open(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn safe_open(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    // Follow-symlink-at-open is unresolved on non-unix; tracked in
    // raikiri-spike-akk.  Regain parity when the follow-up lands.
    std::fs::File::open(path)
}

/// Maximum per-fixture-file size cap (input.html or expected/page-*.png).
///
/// Mirrors [`raikiri_dom::fonts::FONT_SIZE_CAP`] (100 MiB) as a "large but
/// bounded" fixture size — real reference PNGs are well under 10 MiB and
/// input.html payloads are a few KiB, so 100 MiB leaves ample headroom while
/// still bounding attacker-supplied huge files.
///
/// **Threat surface coverage** (raikiri-spike-d9y.6, Codex security finding
/// `0f198c3daa8c8191a18c1a9f16171669`):
///
/// - **symlink → sensitive file / `/dev/zero`**: `symlink_metadata` +
///   `is_symlink()` reject on both `input.html` and each
///   `expected/page-*.png`, plus an `expected/`-as-symlink pre-check
/// - **direct char/block device / FIFO placement** (e.g. attacker `mkfifo
///   input.html` or `mkfifo expected/page-0000.png`): `!is_file()` gate
///   rejects non-regular files.  `metadata.len()` is unreliable for devices,
///   so this check is load-bearing and cannot be replaced by the size cap
///   alone (mirrors d9y.4's stated `is_file()` reasoning).  The related
///   `expected/`-as-symlink-to-`/dev` vector is closed by a separate
///   `expected/` pre-check in [`load_fixture`], not by this gate.
/// - **oversized regular file** (memory exhaustion): `metadata.len() >
///   FIXTURE_SIZE_CAP` up-front reject
/// - **aggregate memory exhaustion via many valid files** (attacker packs
///   `expected/` with N sub-cap page files, each individually accepted):
///   three-layer defense — canonical-form `page-{N:04}.png` filter (rejects
///   `page-0.png`, `page-00000.png`, etc., before content read),
///   [`MAX_EXPECTED_PAGES`] enumeration count cap, and
///   [`FIXTURE_AGGREGATE_BYTES_CAP`] running-total bytes cap that reliably
///   bounds worst-case in-memory peak during load
/// - **fixture-root symlink** (caller-supplied path is itself a symlink,
///   silently redirecting the containment anchor): pre-check via
///   `symlink_metadata(fixture_dir)` rejects a symlinked root
/// - **path escape via intermediate symlink** (e.g. `expected/` → `/tmp/evil`
///   so `expected/page-0000.png` resolves outside the fixture root):
///   `canonicalize` + `starts_with(canonical_root)` prefix check
/// - **mid-read grow (TOCTOU)**: `File::open + take(cap + 1) + read_to_end`
///   +1-probe pattern (per raikiri-spike-d9y.3) catches files that grow
///   between the `metadata.len()` check and the actual read
const FIXTURE_SIZE_CAP: u64 = 100 * 1024 * 1024;

/// Maximum number of `expected/page-*.png` entries `load_fixture` will read.
///
/// Companion to [`FIXTURE_AGGREGATE_BYTES_CAP`].  The bytes cap is the
/// load-bearing aggregate-memory defense; this count cap bounds the
/// enumeration itself against pathological many-tiny-files fixtures
/// (10 000-canonical-file floods that individually fit under the per-file
/// cap and collectively fit under the bytes cap but still stress the
/// `numbered: Vec<(u32, Vec<u8>)>` accumulator).
///
/// The `expected/page-{N:04}.png` name form limits N to 4 decimal digits
/// (0000-9999); 1 024 is comfortably above realistic fixture sizes (real
/// M1 fixtures are single-digit pages, paged-media exports are hundreds)
/// while low enough to reject in tests without a 10 000-file test setup.
const MAX_EXPECTED_PAGES: usize = 1_024;

/// Maximum total bytes across all `expected/page-*.png` entries.
///
/// Load-bearing aggregate-memory defense (raikiri-spike-d9y.6, Codex gate
/// final review round 2).  Without this, [`MAX_EXPECTED_PAGES`] alone
/// still admits `1 024 * FIXTURE_SIZE_CAP` = ~102 GiB.  Cap set at
/// 256 MiB: 2.5× the per-file cap (a single max-size page must still
/// load), above realistic fixture aggregates (real fixtures are
/// single-digit MiB × single-digit pages), and tight enough that the
/// worst-case expected-page payload during load stays bounded at
/// `FIXTURE_AGGREGATE_BYTES_CAP + FIXTURE_SIZE_CAP` (~356 MiB — the
/// running total plus the last file, which is freed on the aggregate-cap
/// reject).  Total loader memory can add `input_html` (up to
/// `FIXTURE_SIZE_CAP` = 100 MiB) plus `Vec` capacity/structural
/// overhead on top of that; overall worst-case retained is
/// `2 * FIXTURE_SIZE_CAP + FIXTURE_AGGREGATE_BYTES_CAP` ≈ 456 MiB
/// (Codex gate final review round 3 doc-precision note).
///
/// Legitimate fixtures approaching this cap should raise a bd request
/// rather than bypass — the number is deliberately tight against attack.
const FIXTURE_AGGREGATE_BYTES_CAP: u64 = 256 * 1024 * 1024;

/// Loaded state of one `tests/reference/<name>/` directory.
#[non_exhaustive]
#[derive(Debug)]
pub struct Fixture {
    /// Absolute path to the fixture root (e.g. `.../tests/reference/hello-world/`).
    pub root: PathBuf,
    /// Contents of `input.html` — raw bytes (HTML5 tokenizer input is byte-oriented).
    pub input_html: Vec<u8>,
    /// Expected PNG bytes per page, indexed by page number.
    /// Loaded from `expected/page-{N:04}.png`; N must be contiguous from 0.
    /// Empty when `expected/` is missing (permitted for update-goldens mode).
    pub expected_pages: Vec<Vec<u8>>,
}

/// Pixel-comparison tolerance for VRT diff.
///
/// Numeric values match spec §12.7 platform tier definitions:
/// - `EXACT` = Tier 1 (Linux x86_64), pixel-exact.
/// - `TIER2` = Tier 2 (Linux aarch64, macOS), max_delta=1, max_diff=0.1%.
/// - `TIER3` = Tier 3 (Windows), max_delta=2, max_diff=0.5%.
///
/// M1 verifies only `EXACT` — TIER2/TIER3 fields are shaped for M2+ platform
/// matrix but the slow path is not exercised in M1 tests.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Max per-channel absolute delta (0 = pixel-exact).
    pub max_delta: u8,
    /// Max fraction of pixels allowed to differ (0.0 = none, 0.001 = 0.1%).
    pub max_diff_fraction: f32,
}

impl Tolerance {
    /// Pixel-exact tolerance (Tier 1: Linux x86_64).
    pub const EXACT: Self = Self {
        max_delta: 0,
        max_diff_fraction: 0.0,
    };
    /// Tier 2 tolerance (Linux aarch64, macOS): max_delta=1, max_diff=0.1%.
    pub const TIER2: Self = Self {
        max_delta: 1,
        max_diff_fraction: 0.001,
    };
    /// Tier 3 tolerance (Windows): max_delta=2, max_diff=0.5%.
    pub const TIER3: Self = Self {
        max_delta: 2,
        max_diff_fraction: 0.005,
    };
}

/// Structured diagnosis of a failing `compare_png` call.
#[non_exhaustive]
#[derive(Debug)]
pub struct DiffReport {
    /// Index of the page this report describes.
    pub page_index: usize,
    /// Width in pixels of the compared image.
    pub width: u32,
    /// Height in pixels of the compared image.
    pub height: u32,
    /// Total number of pixels that differ beyond tolerance.
    pub mismatched_pixel_count: u64,
    /// Details of the first mismatched pixel encountered, if any.
    pub first_mismatch: Option<PixelMismatch>,
    /// Set by `run_and_compare` when it writes the actual PNG on failure.
    pub actual_png_path: Option<PathBuf>,
    /// Set by `run_and_compare` when it writes the diff visualization on failure.
    pub diff_png_path: Option<PathBuf>,
}

/// A single mismatched pixel discovered during comparison.
///
/// RGBA pixel values are in premultiplied-alpha form (the format
/// `tiny_skia::Pixmap::data()` yields — decode/encode passes through
/// this convention). Semi-transparent pixel diffs may look surprising
/// to a viewer expecting straight-alpha color values.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelMismatch {
    /// X coordinate of the mismatched pixel.
    pub x: u32,
    /// Y coordinate of the mismatched pixel.
    pub y: u32,
    /// Expected RGBA pixel value.
    pub expected: [u8; 4],
    /// Actual RGBA pixel value.
    pub actual: [u8; 4],
}

impl fmt::Display for DiffReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "reference diff on page {} ({}x{}): {} pixel(s) mismatched",
            self.page_index, self.width, self.height, self.mismatched_pixel_count
        )?;
        if let Some(m) = &self.first_mismatch {
            writeln!(
                f,
                "  first mismatch at ({}, {}): expected {:?}, actual {:?}",
                m.x, m.y, m.expected, m.actual
            )?;
        }
        if let Some(p) = &self.actual_png_path {
            writeln!(f, "  actual PNG: {}", p.display())?;
        }
        if let Some(p) = &self.diff_png_path {
            writeln!(f, "  diff PNG:   {}", p.display())?;
        }
        Ok(())
    }
}

/// Errors produced by `load_fixture` (and internally by `run_and_compare`).
#[non_exhaustive]
#[derive(Debug)]
pub enum FixtureError {
    /// An I/O operation on the fixture directory failed.
    ///
    /// Kept as `IoError` (not `Io`) for public API stability across d9y.6;
    /// the sibling workspace convention (`FontError::Io`, `ParseError::Io`)
    /// would prefer bare `Io`, but renaming this variant is a public API
    /// break for `raikiri-vrt` consumers.  A follow-up bd captures the
    /// desired rename bundled with the next coordinated API-break window.
    IoError {
        /// Path being accessed when the error occurred.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// `input.html` was not found under the fixture directory.
    MissingInputHtml {
        /// Fixture directory that was searched.
        fixture_dir: PathBuf,
    },
    /// `expected/` page numbers were not contiguous starting from 0.
    NonContiguousPages {
        /// Fixture directory that was searched.
        fixture_dir: PathBuf,
        /// Page numbers actually found.
        found: Vec<u32>,
    },
    /// A fixture-tree entry (`input.html`, `expected/`, or an
    /// `expected/page-*.png`) is a symlink.  We refuse to follow it so an
    /// attacker-controlled fixture cannot exfiltrate arbitrary local files.
    /// See [`FIXTURE_SIZE_CAP`] for the full threat model
    /// (raikiri-spike-d9y.6, Codex finding `0f198c3daa8c8191a18c1a9f16171669`).
    SymlinkRejected {
        /// Path of the rejected symlink.
        path: PathBuf,
    },
    /// A fixture-tree entry exists but is not a regular file (FIFO, device,
    /// socket, block/char device reached through an intermediate symlink,
    /// etc.).  Rejected up-front — `std::fs::read` on a FIFO with no writer
    /// blocks indefinitely, and `/dev/zero` reads exhaust memory.
    NotRegularFile {
        /// Path of the rejected non-regular entry.
        path: PathBuf,
    },
    /// Post-open, fd-based fstat (`check_open_handle_regular`) found the
    /// descriptor `safe_open` returned does not resolve to a regular file.
    /// Race-free complement to [`FixtureError::NotRegularFile`]: the
    /// pre-open path-based `symlink_metadata` check can observe a regular
    /// file that gets swapped for a FIFO/device before `safe_open` runs;
    /// this check instead consults the inode already bound to the opened
    /// descriptor, so no second path lookup can be raced. Mirrors
    /// `raikiri_dom::fonts::FontReadReject::NotRegularFilePostOpen` (bd
    /// raikiri-spike-f4j, ported here via bd raikiri-spike-y92o).
    ///
    /// Shape divergence from the fonts.rs original, intentional: fonts.rs's
    /// `FontReadReject::NotRegularFilePostOpen` is a bare unit variant
    /// because `FontReadReject` defers attaching `path` to the caller
    /// (`FontWarn`/`FontError` add it later). This crate's `FixtureError`
    /// has no such deferred layer — every existing variant already
    /// self-carries its `path` — so this variant does too, and
    /// `check_open_handle_regular`/`check_open_handle_containment` both
    /// take an extra `path: &Path` parameter (absent from the fonts.rs
    /// originals) purely to construct it.
    NotRegularFilePostOpen {
        /// Path whose opened fd resolved to a non-regular kind.
        path: PathBuf,
    },
    /// A fixture-tree file's size exceeds [`FIXTURE_SIZE_CAP`].  Either the
    /// up-front `metadata.len()` check tripped, or the file grew between the
    /// metadata check and the bounded read (TOCTOU-grow, caught by the
    /// `take(cap + 1)` +1-probe).
    OversizedFixture {
        /// Path of the oversized file.
        path: PathBuf,
        /// Actual size observed (bytes).
        size: u64,
        /// Configured cap (bytes) — currently [`FIXTURE_SIZE_CAP`].
        cap: u64,
    },
    /// After canonicalization, the fixture-tree file resolves outside its
    /// fixture root.  Belt-and-suspenders: given the symlink and
    /// non-regular-file gates upstream, this branch is only reachable if a
    /// future change relaxes those gates — but the containment check is what
    /// makes that safe.
    PathEscape {
        /// Canonicalized target path that fell outside the root.
        canonical: PathBuf,
        /// Canonicalized fixture root that the target should have stayed under.
        root: PathBuf,
    },
    /// Post-open, fd-bound containment recheck (`check_open_handle_containment`)
    /// found the already-opened descriptor resolves outside `canonical_root`.
    /// Unlike [`FixtureError::PathEscape`] (pre-open, path-based, and itself
    /// TOCTOU-vulnerable to a second swap before `safe_open` runs), this
    /// variant is derived from the fd `safe_open` actually returned — on
    /// Linux via `/proc/self/fd/<fd>`, on Apple platforms via
    /// `fcntl(fd, F_GETPATH, ..)` (bd raikiri-spike-895r) — so firing here
    /// means an intermediate-directory swap happened inside the pre-open
    /// canonicalize→safe_open re-resolution window itself. Mirrors
    /// `raikiri_dom::fonts::FontReadReject::PathEscapePostOpen` (bd
    /// raikiri-spike-1ef, source of this pattern; ported here via bd
    /// raikiri-spike-y92o — 1ef's close reason named this file as the
    /// same-shape follow-up).
    PathEscapePostOpen {
        /// fd-derived canonical path (post-open) that fell outside the root.
        canonical: PathBuf,
        /// Canonicalized fixture root that the target should have stayed under.
        root: PathBuf,
    },
    /// `expected/` contained more matching page files than [`MAX_EXPECTED_PAGES`].
    /// Companion defense to [`FixtureError::OversizedFixtureAggregate`] —
    /// this one bounds enumeration count against many-tiny-files attacks;
    /// that one bounds aggregate memory.
    TooManyExpectedPages {
        /// Fixture directory that was searched.
        fixture_dir: PathBuf,
        /// Configured cap ([`MAX_EXPECTED_PAGES`]).
        cap: usize,
    },
    /// Aggregate bytes read across `expected/page-*.png` entries exceeded
    /// [`FIXTURE_AGGREGATE_BYTES_CAP`].  Load-bearing defense against
    /// aggregate memory exhaustion — the per-file cap alone allows
    /// `MAX_EXPECTED_PAGES × FIXTURE_SIZE_CAP` = ~102 GiB before this cap
    /// (raikiri-spike-d9y.6, Codex gate final review concern #1).
    OversizedFixtureAggregate {
        /// Fixture directory that was searched.
        fixture_dir: PathBuf,
        /// Running total (bytes) at the point of rejection.
        total: u64,
        /// Configured cap ([`FIXTURE_AGGREGATE_BYTES_CAP`]).
        cap: u64,
    },
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IoError { path, source } => {
                write!(f, "IO error at {}: {source}", path.display())
            }
            Self::MissingInputHtml { fixture_dir } => {
                write!(f, "input.html missing under {}", fixture_dir.display())
            }
            Self::NonContiguousPages { fixture_dir, found } => {
                write!(
                    f,
                    "expected/ under {} has non-contiguous page numbers: {found:?}",
                    fixture_dir.display()
                )
            }
            Self::SymlinkRejected { path } => {
                write!(
                    f,
                    "fixture entry {} is a symlink (rejected: symlinks in the fixture tree could redirect to arbitrary local files)",
                    path.display()
                )
            }
            Self::NotRegularFile { path } => {
                write!(
                    f,
                    "fixture entry {} is not a regular file (rejected: FIFO/device/socket reads can block indefinitely or exhaust memory)",
                    path.display()
                )
            }
            Self::NotRegularFilePostOpen { path } => {
                write!(
                    f,
                    "fixture entry {} opened fd resolves to a non-regular file (TOCTOU-swap between pre-open metadata and open)",
                    path.display()
                )
            }
            Self::OversizedFixture { path, size, cap } => {
                write!(
                    f,
                    "fixture file {} is {size} bytes, exceeding cap {cap}",
                    path.display()
                )
            }
            Self::PathEscape { canonical, root } => {
                write!(
                    f,
                    "fixture entry canonicalizes to {} which escapes fixture root {}",
                    canonical.display(),
                    root.display()
                )
            }
            Self::PathEscapePostOpen { canonical, root } => {
                write!(
                    f,
                    "fixture entry's opened fd resolves to {} which escapes fixture root {} (TOCTOU-swap between pre-open canonicalize and safe_open)",
                    canonical.display(),
                    root.display()
                )
            }
            Self::TooManyExpectedPages { fixture_dir, cap } => {
                write!(
                    f,
                    "expected/ under {} has more than {cap} matching page files",
                    fixture_dir.display()
                )
            }
            Self::OversizedFixtureAggregate {
                fixture_dir,
                total,
                cap,
            } => {
                write!(
                    f,
                    "expected/ under {} totals {total} bytes across pages, exceeding aggregate cap {cap}",
                    fixture_dir.display()
                )
            }
        }
    }
}

impl std::error::Error for FixtureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Compare two PNG buffers pixel-wise under a tolerance.
///
/// Returns `Ok(())` when the images match. Returns `Err(DiffReport)` with
/// `page_index = 0` and `actual_png_path = diff_png_path = None`
/// (this primitive does no I/O). Callers that need artifact-writing behavior
/// should use `run_and_compare` instead.
///
/// # Panics
///
/// Panics if either PNG buffer fails to decode. Both inputs should be
/// valid PNG bytes (typically produced by `tiny_skia::Pixmap::encode_png`
/// or `raikiri::html_to_png`, or read from a well-formed fixture).
pub fn compare_png(
    actual_png: &[u8],
    expected_png: &[u8],
    tolerance: Tolerance,
) -> Result<(), DiffReport> {
    let actual = tiny_skia::Pixmap::decode_png(actual_png)
        .expect("compare_png: actual PNG failed to decode (invariant violation)");
    let expected = tiny_skia::Pixmap::decode_png(expected_png)
        .expect("compare_png: expected PNG failed to decode (invariant violation)");
    let (aw, ah) = (actual.width(), actual.height());
    let (ew, eh) = (expected.width(), expected.height());

    if (aw, ah) != (ew, eh) {
        return Err(DiffReport {
            page_index: 0,
            width: ew,
            height: eh,
            mismatched_pixel_count: u64::from(ew) * u64::from(eh),
            first_mismatch: None,
            actual_png_path: None,
            diff_png_path: None,
        });
    }

    let a = actual.data();
    let e = expected.data();

    // EXACT fast path: byte equality skips per-pixel iteration.
    if tolerance == Tolerance::EXACT && a == e {
        return Ok(());
    }

    let mut mismatched: u64 = 0;
    let mut first: Option<PixelMismatch> = None;
    let total_pixels = u64::from(aw) * u64::from(ah);
    let max_delta = i16::from(tolerance.max_delta);

    // chunks_exact(4) + zip + enumerate avoids needless_range_loop (clippy::style)
    // that a manual `for i in 0..(a.len() / 4)` index loop would trigger.
    for (i, (ax, ex)) in a.chunks_exact(4).zip(e.chunks_exact(4)).enumerate() {
        let differs = ax
            .iter()
            .zip(ex.iter())
            .any(|(&av, &ev)| (i16::from(av) - i16::from(ev)).abs() > max_delta);
        if differs {
            mismatched += 1;
            if first.is_none() {
                let i = i as u32;
                let x = i % aw;
                let y = i / aw;
                first = Some(PixelMismatch {
                    x,
                    y,
                    expected: [ex[0], ex[1], ex[2], ex[3]],
                    actual: [ax[0], ax[1], ax[2], ax[3]],
                });
            }
        }
    }

    if mismatched == 0 {
        return Ok(());
    }
    let fraction = (mismatched as f32) / (total_pixels as f32);
    if fraction <= tolerance.max_diff_fraction {
        return Ok(());
    }

    Err(DiffReport {
        page_index: 0,
        width: aw,
        height: ah,
        mismatched_pixel_count: mismatched,
        first_mismatch: first,
        actual_png_path: None,
        diff_png_path: None,
    })
}

/// Post-open fd-based fstat: verify the handle `safe_open` returned still
/// resolves to a regular file. Load-bearing supplement to the pre-open
/// path-based `symlink_metadata` + `is_file()` gate in
/// [`read_bounded_fixture_file`].
///
/// The pre-open path-based check is race-vulnerable — a regular file can be
/// swapped for a FIFO / character device / block device between the
/// pre-open metadata call and the subsequent `safe_open`. `O_NOFOLLOW` does
/// not filter file kind (a FIFO is not a symlink), so the swapped-in kind
/// slips through `safe_open`. Calling `File::metadata()` on the returned fd
/// consults the inode already bound to the descriptor, so no path lookup
/// re-runs and the race window is closed by construction — for the *kind*
/// check.
///
/// This crate's [`safe_open`] pairs `O_NOFOLLOW` with `O_NONBLOCK` — the
/// same pairing `raikiri_dom::fonts::safe_open` carries (bd
/// raikiri-spike-f4j) — so a swapped-in writer-less FIFO returns
/// immediately from `open()` instead of blocking before this fstat is
/// reached. This function closes the *kind*-detection half for all swap
/// classes reachable via `safe_open` — FIFOs, character devices, and block
/// devices alike.
///
/// Mirrors `raikiri_dom::fonts::check_open_handle_regular` (bd
/// raikiri-spike-f4j, source of this pattern; ported here via bd
/// raikiri-spike-y92o; `O_NONBLOCK` pairing added via bd raikiri-spike-z719).
fn check_open_handle_regular(file: &std::fs::File, path: &Path) -> Result<(), FixtureError> {
    // cov:ignore: fstat on a descriptor this function's caller just opened
    // failing (e.g. underlying storage unmounted mid-call) is an
    // environment fault, not a state a deterministic unit test can force.
    // `raikiri_dom::fonts`'s sibling check treats the equivalent as
    // fail-closed `Io`; preserved here as `FixtureError::IoError`.
    let metadata = file.metadata().map_err(|source| FixtureError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(FixtureError::NotRegularFilePostOpen {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Post-open fd-bound containment recheck: verify the descriptor
/// `safe_open` returned still resolves under `canonical_root`, using a
/// path derived **from the fd itself** rather than a fresh path-based
/// `canonicalize` call.
///
/// # Why this closes the intermediate-dir-swap TOCTOU
///
/// [`read_bounded_fixture_file`]'s pre-open gate canonicalizes `path` and
/// checks `starts_with(canonical_root)` *before* calling `safe_open`. Both
/// calls independently re-resolve the same pathname, so they are two
/// separate lookups of a mutable filesystem tree, not one atomic operation
/// on one object: an attacker who swaps an intermediate directory component
/// between the two calls can make the pre-open `canonicalize` observe a
/// path inside `canonical_root` while `safe_open`'s own pathname
/// resolution — running microseconds later — binds the fd to a different,
/// outside-root object. The leaf is still a regular file either way, so
/// neither `check_open_handle_regular` (kind only) nor the pre-open gate
/// (already-passed, stale) catches the swap.
///
/// The fix is to derive the containment check from the **same fd** that
/// will be read, so there is no second pathname resolution after the check
/// to race against. On Linux, `/proc/self/fd/<fd>` is a magic symlink the
/// kernel keeps pointing at the dentry the fd is actually bound to;
/// `readlink`-ing it costs one syscall and does not walk `path` again, so
/// the check is bound to the fd `safe_open` returned rather than to an
/// independent re-resolution of `path`. (Precisely: the kernel reconstructs
/// the *reported path string* from the dentry chain at readlink time, so a
/// rename of an ancestor directory between `safe_open` and this call can
/// still change what `readlink` reports; what cannot happen is a second,
/// attacker-steerable *pathname lookup* of `path` — the class of swap this
/// check closes.)
///
/// # Fail-closed on procfs-unavailable
///
/// If `/proc` is unavailable (e.g. a container/chroot without procfs
/// mounted), `read_link` returns `Err` and this function propagates it via
/// `FixtureError::IoError`. At the [`read_bounded_fixture_file`] callsite
/// that error is **not** skipped — every `FixtureError` this crate produces
/// is `?`-propagated or explicitly returned (never silently dropped and
/// continued past), so there is no warn+skip path anywhere in this crate
/// that could fall back to the weaker pre-open-only guarantee. This is
/// deliberate fail-closed (autonomy.md 原則3): a filesystem that cannot
/// support the authoritative containment check is treated as a hard error.
/// Concretely, on a Linux host without procfs mounted this recheck now
/// hard-fails VRT fixture loading that previously succeeded under the
/// pre-y92o pre-open-only containment check — this environment has
/// `/proc/self/fd/` available (verified while implementing this task), but
/// future readers deploying this crate's tests inside a minimal
/// container/chroot should expect that dependency.
///
/// # Portability
///
/// Two platform families have an active fd-to-path primitive; a third
/// (everything else, including non-Apple BSDs and Windows) has none wired
/// up and falls back to a no-op.
///
/// - **Linux** (this impl): `std::fs::read_link` on `/proc/self/fd/<fd>`
///   (pure Rust, no `unsafe`, no new dependency).
/// - **Apple platforms** (macOS/iOS/tvOS/watchOS/visionOS, see the
///   `#[cfg(any(target_os = "macos", ...))]` impl below): `fcntl(fd,
///   F_GETPATH, ..)` via [`rustix::fs::getpath`], a safe wrapper — added
///   as a direct dependency in bd raikiri-spike-895r per PMO decision
///   (2026-08-10 14:30), mirroring `raikiri_dom::fonts`'s identical arm
///   (bd raikiri-spike-0nww): Pure Rust, already transitively vetted in
///   this dependency tree (`Cargo.lock` carried rustix v1.1.4 via this
///   crate's own `tempfile` dev-dependency before this change), keeps
///   raikiri-vrt's own `unsafe` surface at zero for this fix. `F_GETPATH`
///   is a Darwin/XNU-specific `fcntl` command — it is not a generic BSD
///   primitive; FreeBSD/OpenBSD/NetBSD/DragonFly have no equivalent, and
///   `rustix::fs::getpath` itself is `#[cfg(apple)]`-gated internally
///   (see rustix's own `build.rs`) to exactly this platform set, so this
///   fix covers Apple platforms only.
/// - **Everything else** (non-Apple BSDs, Windows, ...): no-op fallback
///   (below) that leaves the pre-open canonicalize check as the only
///   containment gate there — no worse than before this change.
///   `raikiri_dom::fonts`'s analogous non-Linux, non-Apple gap was
///   researched in bd raikiri-spike-7cz5 (closed, research-only,
///   decomposed per a wall/build PMO-decision escalation); this file's
///   residual on that same platform set remains untracked, same as
///   `fonts.rs`'s.
///
/// Mirrors `raikiri_dom::fonts::check_open_handle_containment` (bd
/// raikiri-spike-1ef, source of this pattern; ported here via bd
/// raikiri-spike-y92o — 1ef's close reason named this file as the
/// same-shape follow-up; Apple-platform arm ported via bd
/// raikiri-spike-895r, mirroring bd raikiri-spike-0nww).
#[cfg(target_os = "linux")]
fn check_open_handle_containment(
    file: &std::fs::File,
    path: &Path,
    canonical_root: &Path,
) -> Result<(), FixtureError> {
    use std::os::unix::io::AsRawFd;
    let fd_link = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    // cov:ignore: readlink on `/proc/self/fd/<fd>` for a descriptor this
    // function's caller just opened failing (procfs unmounted, fd-table
    // race) is an environment fault, not deterministically unit-testable.
    // Mirrors `raikiri_dom::fonts`'s fail-closed `Io` treatment of the
    // equivalent `read_link` call.
    let canonical = std::fs::read_link(&fd_link).map_err(|source| FixtureError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(FixtureError::PathEscapePostOpen {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    Ok(())
}

/// Apple-platform implementation: derives the post-open containment path
/// from the same fd `safe_open` returned, via `fcntl(fd, F_GETPATH, ..)`
/// through [`rustix::fs::getpath`] — the Darwin/XNU analogue of the Linux
/// arm's `/proc/self/fd/<fd>` readlink above. Same race-freedom argument
/// applies: `F_GETPATH` asks the kernel for the path bound to *this*
/// vnode via *this* descriptor, not a fresh pathname lookup of `path`, so
/// there is no second attacker-steerable resolution to race against.
/// (Caveat mirroring the Linux arm's: the kernel reconstructs the
/// *reported path string* from the vnode's cached name cache entry, which
/// can still change if an ancestor directory is renamed, or go stale
/// after the last hard link to the file is unlinked; what cannot happen
/// is a second, attacker-steerable *pathname lookup* of `path` — the
/// class of swap this task closes.)
///
/// A `getpath` failure is mapped through `From<rustix::io::Errno> for
/// std::io::Error` into `FixtureError::IoError` (using `path`, the
/// pre-open pathname, purely to populate that variant's `path` field —
/// same shape as the Linux arm's `read_link` failure mapping above),
/// taking the same fail-closed hard-abort path documented in the
/// "Fail-closed on procfs-unavailable" section above: a filesystem that
/// cannot support the authoritative containment check is a hard error,
/// not a silent fallback to the weaker pre-open-only guarantee. Note the
/// failure *shape* differs from Linux: a live fd's `/proc/self/fd/<fd>`
/// entry effectively cannot fail to `readlink`, while `F_GETPATH`
/// reconstructs its answer from the vnode's name cache and has a genuine
/// (if rare) failure path even for a live, still-open descriptor — the
/// fail-closed mapping above treats that the same as any other
/// containment-check failure.
///
/// # Accepted residual: theoretical panic inside `rustix::fs::getpath`
///
/// `rustix::fs::getpath` (v1.1.4, its only implementation as of this
/// writing, `backend::libc::fs::syscalls::getpath`) allocates a
/// `PATH_MAX`-sized buffer, has the kernel fill it via `fcntl(fd,
/// F_GETPATH, ..)`, then does `buf.iter().position(|&c| c ==
/// 0).unwrap()` to find the NUL terminator — which macOS's documented
/// `F_GETPATH` contract guarantees the kernel writes into a
/// `MAXPATHLEN`-or-larger buffer on success. If a future libc/kernel
/// violated that documented contract — returning success from `fcntl`
/// without writing any NUL into the `PATH_MAX` buffer — that `.unwrap()`
/// would panic rather than return an `Err` this function's `?` could
/// propagate as `FixtureError::IoError`; the subsequent `CString::new(..)
/// .unwrap()` in the same function cannot itself panic, since it runs on
/// the slice already truncated at that same NUL position. This panic
/// path is not reachable through any input this codebase controls (`file`
/// / `canonical_root` do not influence whether the kernel honors its own
/// `F_GETPATH` contract), so it sits outside this function's own
/// fail-closed contract rather than inside it. Identified as PLAUSIBLE
/// non-blocking by the security lens on bd raikiri-spike-0nww (gate 8.2,
/// 2026-08-08) for `raikiri_dom::fonts`'s identical dependency and
/// deferred here to bd raikiri-spike-895r; documented, not mitigated
/// (upstreaming a `rustix` fix is out of this task's scope). Re-verify
/// against the then-current `rustix` version if this dependency is ever
/// bumped past 1.x.
///
/// # Untested in CI
///
/// No CI target for any Apple platform exists in this repo as of
/// 2026-08-10. The unit tests mirroring
/// `check_open_handle_containment_accepts_file_within_root` /
/// `_rejects_file_outside_root` above are compiled and pinned under this
/// same `cfg` below but have never executed against a real toolchain.
/// Accepted per PMO decision on bd raikiri-spike-895r (2026-08-10 14:30),
/// same acceptance shape as `raikiri_dom::fonts`'s sibling arm (bd
/// raikiri-spike-0nww, 2026-08-07 17:37): document untested-status rather
/// than block on standing up Apple CI. If a future Apple CI run fails
/// these tests, the first suspect should be `F_GETPATH`'s path *form*
/// rather than the containment logic — Darwin resolves several common
/// temp-dir prefixes to a different canonical form than the path used to
/// reach them (`/tmp` → `/private/tmp`, `/var` → `/private/var`), and
/// `tempfile::tempdir()` on macOS lands under `/var/folders/...`, i.e.
/// exactly such a prefix. This function itself does not care
/// (`canonical_root` is derived the same way, from `std::fs::canonicalize`
/// on the same temp dir, so both sides of the `starts_with` comparison
/// would resolve consistently), but a mismatch here is the first thing to
/// check before suspecting the containment check proper.
///
/// bd raikiri-spike-895r, mirrors `raikiri_dom::fonts`'s Apple-platform
/// arm (bd raikiri-spike-0nww), follow-up to raikiri-spike-1ef /
/// raikiri-spike-y92o / raikiri-spike-7cz5.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
fn check_open_handle_containment(
    file: &std::fs::File,
    path: &Path,
    canonical_root: &Path,
) -> Result<(), FixtureError> {
    use std::os::unix::ffi::OsStringExt;
    let path_cstr = rustix::fs::getpath(file).map_err(|errno| FixtureError::IoError {
        path: path.to_path_buf(),
        source: std::io::Error::from(errno),
    })?;
    let canonical = PathBuf::from(std::ffi::OsString::from_vec(path_cstr.into_bytes()));
    if !canonical.starts_with(canonical_root) {
        return Err(FixtureError::PathEscapePostOpen {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    Ok(())
}

/// Fallback for every other platform (non-Apple BSDs, Windows, ...): no
/// portable fd-to-path primitive is wired up (see the Portability section
/// on the `target_os = "linux"` impl above). No-op so behavior on these
/// platforms is unchanged by this task — the pre-open `canonicalize` +
/// `starts_with` gate in [`read_bounded_fixture_file`] remains the only
/// containment check, exactly as it was before bd raikiri-spike-y92o.
#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn check_open_handle_containment(
    _file: &std::fs::File,
    _path: &Path,
    _canonical_root: &Path,
) -> Result<(), FixtureError> {
    Ok(())
}

/// Read a single fixture-tree file with the full defense stack:
/// leaf-symlink reject (pre-open metadata + open-time O_NOFOLLOW on unix),
/// `!is_file()` reject, up-front size cap,
/// canonicalize-and-`starts_with(canonical_root)` containment check,
/// `safe_open` at open time, two post-open fd-based rechecks —
/// [`check_open_handle_regular`] (rejects non-regular kinds bound to the
/// descriptor) and [`check_open_handle_containment`] (rejects an fd
/// resolving outside `canonical_root`, on Linux via `/proc/self/fd/<fd>`,
/// on Apple platforms via `fcntl(fd, F_GETPATH, ..)`) — both race-free
/// against a pre-open→open path-swap because they consult the fd
/// `safe_open` actually returned rather than re-resolving `path`, and a
/// bounded `take(cap + 1)` read that also catches TOCTOU-grow.
///
/// The open-time O_NOFOLLOW closes the leaf-swap race between the pre-open
/// `symlink_metadata` check and `File::open` on unix (raikiri-spike-8yu).
/// Non-unix retains follow-at-open semantics; tracked in
/// raikiri-spike-akk.
///
/// The two post-open rechecks port `raikiri_dom::fonts::read_bounded_font_file`'s
/// equivalent stack (bd raikiri-spike-f4j / raikiri-spike-1ef) to this
/// crate via bd raikiri-spike-y92o — 1ef's close reason named this
/// function as the same-shape follow-up that had not yet received them.
///
/// See [`FIXTURE_SIZE_CAP`] for the threat model these layers cover.
fn read_bounded_fixture_file(path: &Path, canonical_root: &Path) -> Result<Vec<u8>, FixtureError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| FixtureError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(FixtureError::SymlinkRejected {
            path: path.to_path_buf(),
        });
    }
    // `is_file()` gate is load-bearing: it catches direct FIFO/device/socket
    // placement (e.g. attacker `mkfifo input.html`) that the symlink gate
    // doesn't cover.  `metadata.len()` reports 0 for devices, so the size cap
    // won't help either — only `is_file()` fails these entries closed.
    // (Intermediate-symlink escape into `/dev` via `expected/`-as-symlink is
    // already blocked by `load_fixture`'s pre-check on `expected/` itself;
    // this gate is not load-bearing for that vector.)
    if !file_type.is_file() {
        return Err(FixtureError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > FIXTURE_SIZE_CAP {
        return Err(FixtureError::OversizedFixture {
            path: path.to_path_buf(),
            size: metadata.len(),
            cap: FIXTURE_SIZE_CAP,
        });
    }
    // Containment check via canonicalization.  Given the symlink and
    // non-regular-file gates upstream, an unresolvable escape via a leaf
    // symlink is already rejected; this branch covers intermediate-symlink
    // escapes (e.g. `expected/` symlinked to `/tmp/evil`) and any future
    // relaxation of the upstream gates.
    let canonical = std::fs::canonicalize(path).map_err(|source| FixtureError::IoError {
        path: path.to_path_buf(),
        source,
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(FixtureError::PathEscape {
            canonical,
            root: canonical_root.to_path_buf(),
        });
    }
    // safe_open adds O_NOFOLLOW on unix so a leaf-symlink swapped in between
    // the earlier symlink_metadata check and this open call cannot cause a
    // fresh symlink target to be followed.  The Err arm's inline comment
    // below documents the errno mapping and the portable fallback that
    // covers legacy BSD variants.  bd raikiri-spike-8yu.
    let mut file = match safe_open(path) {
        Ok(f) => f,
        Err(e) => {
            #[cfg(unix)]
            {
                // POSIX mandates ELOOP for O_NOFOLLOW on symlink (Linux, macOS,
                // and modern FreeBSD comply). Legacy BSDs may return EMLINK or
                // EFTYPE instead; the post-Err symlink_metadata recheck catches
                // those cases portably at the cost of one extra stat syscall on
                // the reject path. Not race-perfect (an attacker could swap the
                // symlink back to a regular file between safe_open and this
                // recheck), but covers the common attack shape while remaining
                // simple. Full inode-verify would require fstat-after-open on the
                // handle safe_open never returned.
                let looks_like_symlink_swap = e.raw_os_error() == Some(libc::ELOOP)
                    || std::fs::symlink_metadata(path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                if looks_like_symlink_swap {
                    return Err(FixtureError::SymlinkRejected {
                        path: path.to_path_buf(),
                    });
                }
            }
            return Err(FixtureError::IoError {
                path: path.to_path_buf(),
                source: e,
            });
        }
    };
    // Post-open fd-based fstat: reject if the descriptor `safe_open` bound
    // does not resolve to a regular file. Closes the *kind*-detection half
    // of the FIFO/device swap TOCTOU window that the pre-open path-based
    // `symlink_metadata` + `is_file()` gate above cannot cover; `safe_open`'s
    // `O_NONBLOCK` (bd raikiri-spike-z719) closes the open-time-block half,
    // so a writer-less FIFO swapped into this window cannot hang `open()`
    // before this fstat runs. See `check_open_handle_regular`'s doc for the
    // full rationale. bd raikiri-spike-f4j (source pattern), raikiri-spike-y92o
    // (this port), raikiri-spike-z719 (O_NONBLOCK closing the residual gap).
    check_open_handle_regular(&file, path)?;
    // Post-open fd-bound containment recheck: verify the descriptor
    // `safe_open` bound still resolves under `canonical_root`, derived from
    // the fd itself (Linux: `/proc/self/fd/<fd>` readlink; Apple platforms:
    // `fcntl(fd, F_GETPATH, ..)`) rather than a fresh path-based
    // `canonicalize`. Closes the intermediate-dir-swap TOCTOU window
    // between the pre-open `canonicalize` above and `safe_open`'s own
    // pathname re-resolution. See `check_open_handle_containment`'s doc
    // for the full rationale and the remaining no-op fallback. bd
    // raikiri-spike-1ef (source pattern), raikiri-spike-y92o (this port),
    // raikiri-spike-895r (Apple-platform arm).
    check_open_handle_containment(&file, path, canonical_root)?;
    read_bounded_from_open_file(&mut file, path, FIXTURE_SIZE_CAP)
}

/// Bounded read on an already-opened file (`+1-probe` post-read length gate,
/// raikiri-spike-d9y.3 pattern, factored out in raikiri-spike-t19 for
/// injectable-cap testing).
///
/// Reads at most `size_cap + 1` bytes and returns `OversizedFixture` when
/// the buffer grew past `size_cap`.  The `+1-probe` is the load-bearing
/// defense that turns a TOCTOU-grow (file grew past the cap between the
/// caller's metadata check and this read) into a hard error instead of a
/// silently truncated buffer.  The t19 regression tests exercise the
/// post-read length gate in isolation via a static oversized file with a
/// small cap; simulating an actual TOCTOU-grow race remains bd
/// raikiri-spike-51n scope.
///
/// `size_cap` is a `u64` parameter (rather than the hard-coded
/// [`FIXTURE_SIZE_CAP`] const) so tests can pass a small cap against tiny
/// test files.  `saturating_add(1)` guards against callers passing
/// `u64::MAX` — the same regression pattern fe1's
/// `raikiri_traits::io::read_bounded_regular_file` codifies.
fn read_bounded_from_open_file(
    file: &mut std::fs::File,
    path: &Path,
    size_cap: u64,
) -> Result<Vec<u8>, FixtureError> {
    let mut bytes = Vec::new();
    file.take(size_cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| FixtureError::IoError {
            path: path.to_path_buf(),
            source,
        })?;
    let len = bytes.len() as u64;
    if len > size_cap {
        return Err(FixtureError::OversizedFixture {
            path: path.to_path_buf(),
            size: len,
            cap: size_cap,
        });
    }
    Ok(bytes)
}

/// Load a `tests/reference/<name>/` fixture directory.
///
/// Reads `input.html` (required) and any `expected/page-{N:04}.png` files.
/// Pages must be numbered contiguously from 0; gaps produce
/// `FixtureError::NonContiguousPages`. Missing `expected/` (or an empty one)
/// is permitted — this is the update-goldens starting state.
///
/// PNG bytes are stored raw; decoding is deferred until `compare_png` runs.
///
/// # Security
///
/// See [`FIXTURE_SIZE_CAP`] and [`read_bounded_fixture_file`] for the
/// defense stack against untrusted fixture trees (raikiri-spike-d9y.6).
/// Briefly: fixture-root-symlink reject, leaf-symlink reject, `!is_file()`
/// reject, per-file 100 MiB size cap, [`MAX_EXPECTED_PAGES`] enumeration
/// count cap, [`FIXTURE_AGGREGATE_BYTES_CAP`] aggregate bytes cap,
/// canonical `page-{N:04}.png` name filter, canonicalized-prefix
/// containment check, and a bounded read that catches TOCTOU-grow.  Symlinks anywhere in the fixture tree — including
/// the fixture-directory anchor itself — are refused even when they'd
/// resolve inside the intended root; this is a deliberate blanket policy
/// (no real reference fixture currently uses symlinks) and tests pin the
/// behavior.
pub fn load_fixture(fixture_dir: &Path) -> Result<Fixture, FixtureError> {
    // Root-symlink pre-check: `canonicalize` follows symlinks silently, so
    // a symlinked fixture root would redirect the containment anchor to the
    // symlink target.  The declared blanket policy is "symlinks anywhere in
    // the fixture tree are refused", and the root is part of that tree, so
    // reject before canonicalize even sees it.  Non-existence is normal
    // (missing input.html shape); other stat errors propagate as IoError.
    match std::fs::symlink_metadata(fixture_dir) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(FixtureError::SymlinkRejected {
                path: fixture_dir.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::MissingInputHtml {
                fixture_dir: fixture_dir.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(FixtureError::IoError {
                path: fixture_dir.to_path_buf(),
                source,
            });
        }
    }

    // Canonicalize the fixture root once as the containment anchor.  If
    // `fixture_dir` itself doesn't exist, preserve the legacy "missing
    // input.html" error shape so existing callers see the same variant.
    let canonical_root = match std::fs::canonicalize(fixture_dir) {
        Ok(p) => p,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::MissingInputHtml {
                fixture_dir: fixture_dir.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(FixtureError::IoError {
                path: fixture_dir.to_path_buf(),
                source,
            });
        }
    };

    let input_path = canonical_root.join("input.html");
    let input_html = match read_bounded_fixture_file(&input_path, &canonical_root) {
        Ok(bytes) => bytes,
        Err(FixtureError::IoError { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Err(FixtureError::MissingInputHtml {
                fixture_dir: fixture_dir.to_path_buf(),
            });
        }
        Err(e) => return Err(e),
    };

    let expected_dir = canonical_root.join("expected");
    // Guard `expected/` itself against symlink shenanigans.  Non-existent is
    // fine (update-goldens starting state); anything present must be a real
    // directory.  Doing the pre-check here means later per-entry symlink
    // handling only has to worry about the child files, not an intermediate
    // symlink at `expected/`.
    let expected_meta = match std::fs::symlink_metadata(&expected_dir) {
        Ok(m) => Some(m),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(FixtureError::IoError {
                path: expected_dir.clone(),
                source,
            });
        }
    };

    let mut numbered: Vec<(u32, Vec<u8>)> = Vec::new();
    // Running aggregate-bytes total across `expected/page-*.png` reads.
    // See `FIXTURE_AGGREGATE_BYTES_CAP`.  Not counting `input.html` — the
    // per-file cap already bounds it and it isn't attacker-multiplied.
    let mut aggregate_bytes: u64 = 0;
    if let Some(meta) = expected_meta {
        let file_type = meta.file_type();
        if file_type.is_symlink() {
            return Err(FixtureError::SymlinkRejected { path: expected_dir });
        }
        if file_type.is_dir() {
            let entries =
                std::fs::read_dir(&expected_dir).map_err(|source| FixtureError::IoError {
                    path: expected_dir.clone(),
                    source,
                })?;
            for entry in entries {
                let entry = entry.map_err(|source| FixtureError::IoError {
                    path: expected_dir.clone(),
                    source,
                })?;
                let path = entry.path();
                let name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                // Match "page-XXXX.png" exactly: 4-digit zero-padded index.
                // Ignore README.md and other files.  Requiring exactly 4
                // digits is load-bearing: without it, `page-0.png`,
                // `page-00.png`, `page-0000.png`, `page-00000.png`, ...
                // all parse to the same index and let an attacker
                // multiplicatively amplify per-file caps with duplicates
                // that only surface as NonContiguousPages *after* every
                // one has been fully read into memory.
                let Some(stem) = name.strip_suffix(".png") else {
                    continue;
                };
                let Some(num_str) = stem.strip_prefix("page-") else {
                    continue;
                };
                if num_str.len() != 4 || !num_str.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                let Ok(n) = num_str.parse::<u32>() else {
                    continue;
                };
                // Aggregate cap #1 (count): reject before spending memory
                // on the (cap + 1)th page.  See MAX_EXPECTED_PAGES.
                if numbered.len() >= MAX_EXPECTED_PAGES {
                    return Err(FixtureError::TooManyExpectedPages {
                        fixture_dir: fixture_dir.to_path_buf(),
                        cap: MAX_EXPECTED_PAGES,
                    });
                }
                let bytes = read_bounded_fixture_file(&path, &canonical_root)?;
                // Aggregate cap #2 (bytes): tally after read so the file
                // just consumed drops on rejection (peak momentary memory
                // = cap + FIXTURE_SIZE_CAP).  saturating_add can't
                // realistically saturate here — per-file cap is 100 MiB
                // and we've already reject-early via numbered.len() — but
                // use saturating semantics as defense-in-depth against a
                // future per-file cap relaxation.
                aggregate_bytes = aggregate_bytes.saturating_add(bytes.len() as u64);
                if aggregate_bytes > FIXTURE_AGGREGATE_BYTES_CAP {
                    return Err(FixtureError::OversizedFixtureAggregate {
                        fixture_dir: fixture_dir.to_path_buf(),
                        total: aggregate_bytes,
                        cap: FIXTURE_AGGREGATE_BYTES_CAP,
                    });
                }
                numbered.push((n, bytes));
            }
        }
        // If neither symlink nor dir (e.g. `expected/` is a plain file),
        // treat it the same as "missing expected/" — no pages to enumerate.
        // Callers already tolerate an empty expected_pages vector.
    }

    numbered.sort_by_key(|(n, _)| *n);
    let found: Vec<u32> = numbered.iter().map(|(n, _)| *n).collect();
    for (i, n) in found.iter().enumerate() {
        if *n as usize != i {
            return Err(FixtureError::NonContiguousPages {
                fixture_dir: fixture_dir.to_path_buf(),
                found,
            });
        }
    }
    let expected_pages = numbered.into_iter().map(|(_, bytes)| bytes).collect();

    Ok(Fixture {
        // Deliberate: keep the caller's supplied path (not canonical_root).
        // Downstream diff-artifact paths and error messages read more
        // naturally as the fixture the caller pointed at, and no consumer
        // relies on `root` being canonicalized.
        root: fixture_dir.to_path_buf(),
        input_html,
        expected_pages,
    })
}

/// Environment variable that switches `run_and_compare` from compare-mode
/// to golden-update mode. Any non-empty value activates update mode; the
/// exact syntax `RAIKIRI_UPDATE_GOLDENS=1` is the documented convention.
///
/// Corresponds to spec §12.7's `cargo test -- --update-goldens` intent;
/// the env-var mechanism is the M1 implementation (see spec §13 drift).
pub const UPDATE_GOLDENS_ENV: &str = "RAIKIRI_UPDATE_GOLDENS";

/// Load a fixture, invoke `pipeline` on its input, and compare or update
/// golden PNGs.
///
/// # Behavior
///
/// - `RAIKIRI_UPDATE_GOLDENS` set: overwrites `expected/` from pipeline
///   output. Any existing `expected/` is deleted first (prevents stale
///   pages when the page count decreases).
/// - Env unset (default): compares each pipeline page against
///   `expected/page-{N:04}.png` under `tolerance`. On failure, writes
///   `<diff_dir_root()>/<fixture_name>/page-{N:04}-{actual,diff}.png`
///   and panics with a formatted DiffReport. In practice `diff_dir_root()`
///   resolves to `target/reference-diffs` (see its doc comment for why).
///
/// # Panics
///
/// - `load_fixture` fails (bad fixture layout).
/// - Pipeline output page count differs from expected page count (compare mode only).
/// - `compare_png` returns a `DiffReport`.
/// - I/O failure writing goldens (update mode) or diff artifacts (compare mode).
/// - fixture_dir has no valid UTF-8 file name (used to name the diff-artifact subdirectory).
/// - I/O failure during golden-update mode (removing or recreating expected/, writing individual page PNGs).
pub fn run_and_compare<F>(fixture_dir: &Path, tolerance: Tolerance, pipeline: F)
where
    F: FnOnce(&[u8]) -> Vec<Vec<u8>>,
{
    let fixture = load_fixture(fixture_dir).unwrap_or_else(|e| panic!("load_fixture failed: {e}"));
    let actual_pages = pipeline(&fixture.input_html);

    if std::env::var(UPDATE_GOLDENS_ENV).is_ok_and(|v| !v.is_empty()) {
        update_goldens(fixture_dir, &actual_pages);
        return;
    }

    if actual_pages.len() != fixture.expected_pages.len() {
        panic!(
            "page count mismatch for {}: expected {}, actual {}",
            fixture_dir.display(),
            fixture.expected_pages.len(),
            actual_pages.len(),
        );
    }

    let fixture_name = fixture_dir
        .file_name()
        .and_then(|s| s.to_str())
        .expect("fixture_dir must have a valid UTF-8 file name")
        .to_string();

    for (page_idx, (actual_png, expected_png)) in actual_pages
        .iter()
        .zip(fixture.expected_pages.iter())
        .enumerate()
    {
        match compare_png(actual_png, expected_png, tolerance) {
            Ok(()) => continue,
            Err(mut report) => {
                report.page_index = page_idx;
                let diff_root = diff_dir_root().join(&fixture_name);
                let (actual_path, diff_path) =
                    write_diff_artifacts(&diff_root, page_idx, actual_png, expected_png);
                report.actual_png_path = actual_path;
                report.diff_png_path = diff_path;
                panic!("{report}");
            }
        }
    }
}

fn update_goldens(fixture_dir: &Path, actual_pages: &[Vec<u8>]) {
    let expected_dir = fixture_dir.join("expected");
    if expected_dir.exists() {
        std::fs::remove_dir_all(&expected_dir)
            .unwrap_or_else(|e| panic!("remove_dir_all({}) failed: {e}", expected_dir.display()));
    }
    std::fs::create_dir_all(&expected_dir)
        .unwrap_or_else(|e| panic!("create_dir_all({}) failed: {e}", expected_dir.display()));
    for (n, png) in actual_pages.iter().enumerate() {
        let path = expected_dir.join(format!("page-{n:04}.png"));
        std::fs::write(&path, png)
            .unwrap_or_else(|e| panic!("write({}) failed: {e}", path.display()));
    }
    eprintln!(
        "raikiri-vrt: updated {} golden(s) for {}",
        actual_pages.len(),
        fixture_dir.display()
    );
}

/// Resolve the base directory for diff artifacts.
/// Prefers `CARGO_TARGET_TMPDIR`, falls back to `CARGO_TARGET_DIR`, then
/// `target/`.
///
/// Note: both `CARGO_TARGET_TMPDIR` and `CARGO_TARGET_DIR` are cargo
/// variables set at *compile time* for the crate being built (readable via
/// `env!()` in source that cargo compiles as a test/bench target); they are
/// not propagated into the *runtime* process environment of the resulting
/// binary. Since this function runs at runtime inside library code, both
/// `std::env::var` lookups below will typically miss, and the `target/`
/// fallback (relative to the process's current directory) is what actually
/// gets used in practice under `cargo test`.
fn diff_dir_root() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_TARGET_TMPDIR") {
        return PathBuf::from(p).join("reference-diffs");
    }
    if let Ok(p) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(p).join("reference-diffs");
    }
    PathBuf::from("target").join("reference-diffs")
}

/// Write `page-{N:04}-actual.png` and `page-{N:04}-diff.png` to `dir` and
/// return the two paths. Returns `(None, None)` on I/O failure — the caller
/// still panics with the DiffReport, but path fields stay unset to signal
/// that the artifacts weren't produced.
fn write_diff_artifacts(
    dir: &Path,
    page_idx: usize,
    actual_png: &[u8],
    expected_png: &[u8],
) -> (Option<PathBuf>, Option<PathBuf>) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!(
            "raikiri-vrt: failed to create diff dir {}: {e} — diff artifacts skipped",
            dir.display()
        );
        return (None, None);
    }
    let actual_path = dir.join(format!("page-{page_idx:04}-actual.png"));
    let diff_path = dir.join(format!("page-{page_idx:04}-diff.png"));

    let actual_written = std::fs::write(&actual_path, actual_png).is_ok();

    let diff_ok = build_and_write_diff(&diff_path, actual_png, expected_png);

    (
        if actual_written {
            Some(actual_path)
        } else {
            None
        },
        if diff_ok { Some(diff_path) } else { None },
    )
}

/// Build a magenta-on-dimmed visualization: pixels that differ appear as
/// solid magenta (255, 0, 255, 255); pixels that match appear as a 40%-dim
/// greyscale of the actual image (helps orient the diff over image content).
fn build_and_write_diff(path: &Path, actual_png: &[u8], expected_png: &[u8]) -> bool {
    let Ok(actual) = tiny_skia::Pixmap::decode_png(actual_png) else {
        return false;
    };
    let Ok(expected) = tiny_skia::Pixmap::decode_png(expected_png) else {
        return false;
    };
    if (actual.width(), actual.height()) != (expected.width(), expected.height()) {
        // Cannot build a pixel-aligned diff for mismatched dimensions.
        return false;
    }
    let w = actual.width();
    let h = actual.height();
    let a = actual.data();
    let e = expected.data();
    let mut out = Vec::with_capacity(a.len());
    for (ax, ex) in a.chunks_exact(4).zip(e.chunks_exact(4)) {
        if ax == ex {
            let y = ((u32::from(ax[0]) * 299 + u32::from(ax[1]) * 587 + u32::from(ax[2]) * 114)
                / 1000) as u8;
            let dim = y / 5 * 2; // ~40% brightness
            out.extend_from_slice(&[dim, dim, dim, 255]);
        } else {
            out.extend_from_slice(&[255, 0, 255, 255]);
        }
    }
    let png = crate::encode_png(&out, w, h);
    std::fs::write(path, png).is_ok()
}

#[cfg(test)]
mod type_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn tolerance_constants_are_distinct() {
        assert_eq!(Tolerance::EXACT.max_delta, 0);
        assert_eq!(Tolerance::EXACT.max_diff_fraction, 0.0);
        assert_eq!(Tolerance::TIER2.max_delta, 1);
        assert!((Tolerance::TIER2.max_diff_fraction - 0.001).abs() < f32::EPSILON);
        assert_eq!(Tolerance::TIER3.max_delta, 2);
        assert!((Tolerance::TIER3.max_diff_fraction - 0.005).abs() < f32::EPSILON);
    }

    #[test]
    fn diff_report_display_contains_key_info() {
        let report = DiffReport {
            page_index: 0,
            width: 100,
            height: 50,
            mismatched_pixel_count: 42,
            first_mismatch: Some(PixelMismatch {
                x: 10,
                y: 20,
                expected: [255, 0, 0, 255],
                actual: [0, 255, 0, 255],
            }),
            actual_png_path: Some(PathBuf::from("/tmp/actual.png")),
            diff_png_path: Some(PathBuf::from("/tmp/diff.png")),
        };
        let s = format!("{report}");
        assert!(s.contains("page 0"), "missing page index: {s}");
        assert!(s.contains("100x50"), "missing dimensions: {s}");
        assert!(s.contains("42"), "missing mismatch count: {s}");
        assert!(s.contains("(10, 20)"), "missing first mismatch coords: {s}");
        assert!(s.contains("actual.png"), "missing actual path: {s}");
        assert!(s.contains("diff.png"), "missing diff path: {s}");
    }

    #[test]
    fn fixture_error_is_error_trait() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<FixtureError>();
    }

    /// `Display` for the two post-open recheck variants added by bd
    /// raikiri-spike-y92o. Constructed directly (no filesystem I/O) since
    /// these are pure formatting checks — the variants' actual construction
    /// sites are pinned by `defense_tests`'s `check_open_handle_regular_*`
    /// / `check_open_handle_containment_*` tests.
    #[test]
    fn fixture_error_not_regular_file_post_open_display_contains_path() {
        let err = FixtureError::NotRegularFilePostOpen {
            path: PathBuf::from("/tmp/evil.bin"),
        };
        let s = err.to_string();
        assert!(s.contains("/tmp/evil.bin"), "missing path: {s}");
        assert!(s.contains("non-regular"), "missing kind description: {s}");
    }

    #[test]
    fn fixture_error_path_escape_post_open_display_contains_paths() {
        let err = FixtureError::PathEscapePostOpen {
            canonical: PathBuf::from("/outside/leaf.bin"),
            root: PathBuf::from("/fixture/root"),
        };
        let s = err.to_string();
        assert!(s.contains("/outside/leaf.bin"), "missing canonical: {s}");
        assert!(s.contains("/fixture/root"), "missing root: {s}");
    }
}

/// Regression tests for the `load_fixture` defense stack introduced in
/// raikiri-spike-d9y.6 (Codex security finding
/// `0f198c3daa8c8191a18c1a9f16171669`).  See [`FIXTURE_SIZE_CAP`] for the
/// full threat-model breakdown.
///
/// Symlink-based tests are gated on `#[cfg(unix)]` because
/// `std::os::unix::fs::symlink` isn't cross-platform.  The non-symlink
/// tests (oversized, happy path, `expected/`-not-a-dir) run everywhere.
#[cfg(test)]
mod defense_tests {
    use super::*;
    use std::io::Write;

    /// 1x1 valid PNG (transparent black), for happy-path fixtures where
    /// we only care that bytes are read faithfully (compare_png doesn't
    /// run in these tests).  Bytes from a small hand-generated PNG.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // signature
        0x00, 0x00, 0x00, 0x0d, // IHDR length
        0x49, 0x48, 0x44, 0x52, // "IHDR"
        0x00, 0x00, 0x00, 0x01, // width 1
        0x00, 0x00, 0x00, 0x01, // height 1
        0x08, 0x06, 0x00, 0x00, 0x00, // bit depth 8, color type RGBA, ...
        0x1f, 0x15, 0xc4, 0x89, // IHDR CRC
        0x00, 0x00, 0x00, 0x0a, // IDAT length
        0x49, 0x44, 0x41, 0x54, // "IDAT"
        0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, // IDAT
        0x0d, 0x0a, 0x2d, 0xb4, // IDAT CRC
        0x00, 0x00, 0x00, 0x00, // IEND length
        0x49, 0x45, 0x4e, 0x44, // "IEND"
        0xae, 0x42, 0x60, 0x82, // IEND CRC
    ];

    fn write_input_html(dir: &Path, body: &[u8]) {
        let mut f = std::fs::File::create(dir.join("input.html")).unwrap();
        f.write_all(body).unwrap();
    }

    fn write_expected_png(dir: &Path, idx: usize) {
        let expected = dir.join("expected");
        std::fs::create_dir_all(&expected).unwrap();
        let path = expected.join(format!("page-{idx:04}.png"));
        std::fs::write(&path, TINY_PNG).unwrap();
    }

    #[test]
    fn happy_path_load_succeeds_after_defense_added() {
        // Baseline: a well-formed fixture still loads through the new
        // symlink / size / containment / bounded-read stack.  If this
        // fails, the defense over-rejects legitimate fixtures.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html><html><body>ok</body></html>");
        write_expected_png(tmp.path(), 0);
        write_expected_png(tmp.path(), 1);

        let fixture = load_fixture(tmp.path()).expect("well-formed fixture must load");
        assert!(fixture.input_html.starts_with(b"<!doctype html>"));
        assert_eq!(fixture.expected_pages.len(), 2);
        assert_eq!(fixture.expected_pages[0], TINY_PNG);
        assert_eq!(fixture.expected_pages[1], TINY_PNG);
    }

    #[test]
    fn oversized_input_html_is_rejected() {
        // TOCTOU-independent path: an up-front `metadata.len() > cap`
        // check trips before we even open the file.  Using a sparse file
        // (`set_len(cap + 1)`) so we don't actually spend 100 MiB of disk.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("input.html");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(FIXTURE_SIZE_CAP + 1).unwrap();
        drop(f);
        write_expected_png(tmp.path(), 0);

        match load_fixture(tmp.path()) {
            Err(FixtureError::OversizedFixture { path: p, size, cap }) => {
                assert_eq!(p.file_name().and_then(|f| f.to_str()), Some("input.html"));
                assert_eq!(size, FIXTURE_SIZE_CAP + 1);
                assert_eq!(cap, FIXTURE_SIZE_CAP);
            }
            other => panic!("expected OversizedFixture, got {other:?}"),
        }
    }

    #[test]
    fn oversized_expected_png_is_rejected() {
        // Same size cap applies to expected/page-*.png entries.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        let png_path = expected.join("page-0000.png");
        let f = std::fs::File::create(&png_path).unwrap();
        f.set_len(FIXTURE_SIZE_CAP + 1).unwrap();
        drop(f);

        match load_fixture(tmp.path()) {
            Err(FixtureError::OversizedFixture { path: p, size, cap }) => {
                assert_eq!(
                    p.file_name().and_then(|f| f.to_str()),
                    Some("page-0000.png")
                );
                assert_eq!(size, FIXTURE_SIZE_CAP + 1);
                assert_eq!(cap, FIXTURE_SIZE_CAP);
            }
            other => panic!("expected OversizedFixture, got {other:?}"),
        }
    }

    #[test]
    fn size_cap_boundary_is_accepted() {
        // Silent over-reject canary (mirrors d9y.4 boundary test): a file
        // whose size is exactly FIXTURE_SIZE_CAP must load — the check is
        // `>` cap, not `>=`.  Sparse file keeps disk usage minimal.
        let tmp = tempfile::tempdir().unwrap();
        // Small input.html so we don't spend 100 MiB there too.
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        let png_path = expected.join("page-0000.png");
        let f = std::fs::File::create(&png_path).unwrap();
        f.set_len(FIXTURE_SIZE_CAP).unwrap();
        drop(f);

        let fixture = load_fixture(tmp.path())
            .expect("boundary-size (== FIXTURE_SIZE_CAP) fixture must load");
        // Sparse file: reads back as `FIXTURE_SIZE_CAP` zero bytes.  Verify
        // length only (allocating a 100 MiB assertion buffer just to
        // compare would double the memory footprint of the test needlessly).
        assert_eq!(fixture.expected_pages.len(), 1);
        assert_eq!(fixture.expected_pages[0].len() as u64, FIXTURE_SIZE_CAP);
    }

    #[test]
    fn read_bounded_from_open_file_trips_plus1_probe_on_oversized_read() {
        // raikiri-spike-t19: pins the +1-probe post-read reject in
        // `read_bounded_from_open_file`, AND pins the `+1` bound itself.
        //
        // The file is deliberately much larger than `cap + 1` (20 bytes
        // for cap=8, so 12 bytes past the probe limit) and the assertion
        // is `size == cap + 1` (exactly).  This shape catches two
        // regression classes at once:
        //
        //   1. Removing the post-read `bytes.len() > cap` check
        //      (`OversizedFixture` never fires → test enters the panic
        //      arm).
        //   2. Widening the read bound to more than `cap + 1` — e.g. a
        //      typo `take(cap + 2)` or `take(u64::MAX)` — which would
        //      read 20 bytes into `bytes`, still trigger the reject,
        //      but with `size = 20 ≠ cap + 1` → the equality assert
        //      fails.  A cap+1-sized file would let both regressions
        //      pass the reject arm silently (bd raikiri-spike-t19
        //      §8.3 codex round 1 finding).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many_bytes.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"aaaaaaaaaaaaaaaaaaaa") // 20 bytes, well over cap + 1
            .unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        let cap: u64 = 8;

        match read_bounded_from_open_file(&mut file, &path, cap) {
            Err(FixtureError::OversizedFixture {
                path: p,
                size,
                cap: c,
            }) => {
                assert_eq!(p, path);
                assert_eq!(
                    size,
                    cap + 1,
                    "size must be exactly cap + 1 — pins the +1 bound; \
                     a widened read (e.g. take(cap + 2)) would report size > cap + 1"
                );
                assert_eq!(c, cap);
            }
            other => {
                panic!("expected OversizedFixture via +1-probe post-read reject, got {other:?}")
            }
        }
    }

    #[test]
    fn read_bounded_from_open_file_accepts_at_boundary_cap() {
        // raikiri-spike-t19 companion pin: silent over-reject canary for
        // the injectable-cap helper.  A file of exactly `cap` bytes must
        // load successfully — the post-read check is `>` cap, not `>=`,
        // and the `take(cap + 1)` read yields exactly `cap` bytes when
        // the file is not growing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eight_bytes.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"aaaaaaaa") // 8 bytes
            .unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        let cap: u64 = 8;

        let bytes = read_bounded_from_open_file(&mut file, &path, cap)
            .expect("boundary-size (== cap) read must accept");
        assert_eq!(bytes, b"aaaaaaaa");
    }

    #[test]
    fn expected_dir_as_plain_file_is_ignored_not_read() {
        // If `expected/` is a plain regular file (not a dir), we treat it
        // the same as "no expected/ at all" — no pages to enumerate, no
        // read attempted on it.  This documents the fallback: the
        // filesystem type check is up-front, before any expensive read.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        std::fs::write(tmp.path().join("expected"), b"not a directory").unwrap();

        let fixture =
            load_fixture(tmp.path()).expect("fixture with plain-file `expected` should still load");
        assert!(fixture.expected_pages.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn symlink_input_html_to_dev_zero_is_rejected() {
        // Core threat: attacker replaces `input.html` with a symlink to
        // `/dev/zero`.  Old `std::fs::read` would follow it and read
        // forever, exhausting memory.  New defense: `symlink_metadata` +
        // `is_symlink()` rejects at the leaf, before any read.
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("input.html");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        write_expected_png(tmp.path(), 0);

        match load_fixture(tmp.path()) {
            Err(FixtureError::SymlinkRejected { path }) => {
                assert_eq!(
                    path.file_name().and_then(|f| f.to_str()),
                    Some("input.html")
                );
            }
            other => panic!("expected SymlinkRejected for /dev/zero symlink, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlink_expected_png_to_sensitive_file_is_rejected() {
        // Second-file variant: attacker replaces a page PNG with a symlink
        // to a sensitive local file.  Same leaf-symlink reject applies.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        // Target need only exist; use /etc/hostname (readable on virtually
        // any unix host, small, non-sensitive to actually leak in a test).
        std::os::unix::fs::symlink("/etc/hostname", expected.join("page-0000.png")).unwrap();

        match load_fixture(tmp.path()) {
            Err(FixtureError::SymlinkRejected { path }) => {
                assert_eq!(
                    path.file_name().and_then(|f| f.to_str()),
                    Some("page-0000.png")
                );
            }
            other => panic!("expected SymlinkRejected for expected/ png symlink, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn path_traversal_via_expected_symlink_is_rejected() {
        // `expected/` itself is a symlink pointing outside the fixture
        // root.  The `expected/` pre-check catches this as
        // SymlinkRejected before we even try to enumerate.  Per the
        // advisor: don't lock in a specific variant here — the important
        // property is "not accepted"; any of the three
        // security-related variants (SymlinkRejected / NotRegularFile /
        // PathEscape) is a valid rejection.
        let tmp_fixture = tempfile::tempdir().unwrap();
        let tmp_outside = tempfile::tempdir().unwrap();
        write_input_html(tmp_fixture.path(), b"<!doctype html>");
        // Populate outside/ with a PNG so if the reject didn't fire we'd
        // actually escape and read the outside file.
        std::fs::write(tmp_outside.path().join("page-0000.png"), TINY_PNG).unwrap();
        std::os::unix::fs::symlink(tmp_outside.path(), tmp_fixture.path().join("expected"))
            .unwrap();

        match load_fixture(tmp_fixture.path()) {
            Err(FixtureError::SymlinkRejected { .. })
            | Err(FixtureError::NotRegularFile { .. })
            | Err(FixtureError::PathEscape { .. }) => {
                // any of the containment gates firing is a correct reject
            }
            other => {
                panic!("expected traversal reject (Symlink/NotRegular/PathEscape), got {other:?}")
            }
        }
    }

    /// Run `load_fixture(dir)` on a worker thread and fail-fast on timeout.
    /// Prevents FIFO-regression tests from silently hanging the test suite
    /// forever if the `!is_file()` gate ever regresses — in that case
    /// `File::open`/`read_to_end` would block on the FIFO with no writer.
    #[cfg(unix)]
    fn load_fixture_with_watchdog(
        dir: PathBuf,
        timeout: std::time::Duration,
    ) -> Result<Fixture, FixtureError> {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = tx.send(load_fixture(&dir));
        });
        match rx.recv_timeout(timeout) {
            Ok(result) => {
                // Best-effort join; thread has already sent its result.
                let _ = handle.join();
                result
            }
            Err(_) => panic!(
                "load_fixture did not return within {:?} — FIFO gate has likely regressed \
                 (File::open on a FIFO with no writer blocks indefinitely). \
                 Leaving worker thread detached so the suite fails fast rather than hanging.",
                timeout
            ),
        }
    }

    #[test]
    #[cfg(unix)]
    fn fifo_as_input_html_is_rejected_not_blocked() {
        // If an attacker places `input.html` as a FIFO with no writer,
        // `File::open` + `read_to_end` blocks indefinitely.  The `!is_file()`
        // gate rejects the FIFO up-front so the test process cannot hang.
        // A watchdog wrapper converts a regression from "test hangs forever"
        // into "test panics after 2 s with a clear message".
        let tmp = tempfile::tempdir().unwrap();
        let fifo = tmp.path().join("input.html");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());
        write_expected_png(tmp.path(), 0);

        match load_fixture_with_watchdog(
            tmp.path().to_path_buf(),
            std::time::Duration::from_secs(2),
        ) {
            Err(FixtureError::NotRegularFile { path }) => {
                assert_eq!(
                    path.file_name().and_then(|f| f.to_str()),
                    Some("input.html")
                );
            }
            other => panic!("expected NotRegularFile for FIFO input.html, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn fifo_at_expected_png_is_rejected_not_blocked() {
        // Parallel to `fifo_as_input_html_is_rejected_not_blocked`, but
        // pins the `!is_file()` gate for the second consumer of
        // `read_bounded_fixture_file` — the `expected/page-*.png`
        // enumeration path.  Same watchdog + same regression semantics:
        // a gate regression here would `File::open` the FIFO and block.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        let fifo = expected.join("page-0000.png");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        match load_fixture_with_watchdog(
            tmp.path().to_path_buf(),
            std::time::Duration::from_secs(2),
        ) {
            Err(FixtureError::NotRegularFile { path }) => {
                assert_eq!(
                    path.file_name().and_then(|f| f.to_str()),
                    Some("page-0000.png")
                );
            }
            other => panic!("expected NotRegularFile for FIFO page-0000.png, got {other:?}"),
        }
    }

    #[test]
    fn zero_byte_input_html_and_expected_png_load_successfully() {
        // Boundary canary at the low end: a zero-byte file should load
        // (`.take(cap + 1).read_to_end` yields 0 bytes; `is_file()` still
        // true; size 0 ≤ cap).  Pins current behavior so a future hygiene
        // check (`if metadata.len() == 0 { reject }`) doesn't silently
        // change fixture semantics.  Cheap counterpart to
        // `size_cap_boundary_is_accepted` at the high end.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        std::fs::write(expected.join("page-0000.png"), b"").unwrap();

        let fixture =
            load_fixture(tmp.path()).expect("zero-byte input.html + zero-byte page should load");
        assert!(fixture.input_html.is_empty());
        assert_eq!(fixture.expected_pages.len(), 1);
        assert!(fixture.expected_pages[0].is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn fixture_root_as_symlink_is_rejected() {
        // Codex gate final review finding: `canonicalize(fixture_dir)`
        // silently follows a symlinked root, redirecting the containment
        // anchor to the target directory.  The declared blanket policy
        // ("symlinks anywhere in the fixture tree are refused") must
        // include the root itself.  Pre-check via `symlink_metadata` on
        // the fixture root rejects with SymlinkRejected before
        // canonicalize sees it.
        let tmp_real = tempfile::tempdir().unwrap();
        write_input_html(tmp_real.path(), b"<!doctype html>");
        write_expected_png(tmp_real.path(), 0);

        let tmp_parent = tempfile::tempdir().unwrap();
        let link_root = tmp_parent.path().join("root-link");
        std::os::unix::fs::symlink(tmp_real.path(), &link_root).unwrap();

        match load_fixture(&link_root) {
            Err(FixtureError::SymlinkRejected { path }) => {
                assert_eq!(path, link_root);
            }
            other => panic!("expected SymlinkRejected for symlinked root, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_page_indices_via_noncanonical_names_are_ignored() {
        // Codex gate final review finding: `page-0.png`, `page-00.png`,
        // `page-0000.png`, `page-00000.png` all parse to index 0.  Before
        // the canonical-form filter, an attacker could pack N copies with
        // different padding widths; each would pass per-file caps and be
        // fully read into `numbered` before duplicate detection surfaced
        // as NonContiguousPages — multiplying the per-file cap by the
        // padding-width space.  Fix: require exactly 4 zero-padded digits
        // in the enumeration filter; noncanonical forms are skipped
        // (never opened, never read).  Test: place `page-0000.png` +
        // several noncanonical duplicates; load should succeed with
        // exactly one page (the canonical one).
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        write_expected_png(tmp.path(), 0);
        let expected = tmp.path().join("expected");
        // Noncanonical padding widths — all would parse to n=0.
        // Populate with plausibly-huge fake bodies so a regression
        // (reading them) would be easy to spot in mem accounting.
        for name in ["page-0.png", "page-00.png", "page-00000.png"] {
            std::fs::write(expected.join(name), vec![0u8; 8]).unwrap();
        }
        // Non-digit padding — should also be skipped.
        std::fs::write(expected.join("page-000a.png"), vec![0u8; 8]).unwrap();

        let fixture = load_fixture(tmp.path())
            .expect("canonical page-0000.png with noncanonical siblings should load");
        assert_eq!(
            fixture.expected_pages.len(),
            1,
            "only the canonical page should enumerate"
        );
        assert_eq!(fixture.expected_pages[0], TINY_PNG);
    }

    #[test]
    fn too_many_expected_pages_is_rejected_before_read() {
        // Codex gate final review finding: aggregate memory is
        // unbounded — an attacker packing `expected/` with many valid
        // regular files, each ≤ FIXTURE_SIZE_CAP, can OOM even with the
        // per-file cap.  Fix: MAX_EXPECTED_PAGES aggregate count cap
        // checked before the (cap+1)th page is read.
        //
        // Place MAX_EXPECTED_PAGES + 1 canonical zero-byte pages: the
        // walker enumerates them all (no I/O for content yet — just
        // `read_dir`), each canonical name passes the syntactic filter,
        // and the cap check `numbered.len() >= MAX_EXPECTED_PAGES` fires
        // when the loop attempts to read the (cap+1)th entry.  Zero-byte
        // files keep disk + total test cost minimal.
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        for n in 0..=MAX_EXPECTED_PAGES {
            std::fs::write(expected.join(format!("page-{n:04}.png")), b"").unwrap();
        }

        match load_fixture(tmp.path()) {
            Err(FixtureError::TooManyExpectedPages { fixture_dir, cap }) => {
                assert_eq!(fixture_dir, tmp.path());
                assert_eq!(cap, MAX_EXPECTED_PAGES);
            }
            other => panic!("expected TooManyExpectedPages, got {other:?}"),
        }
    }

    #[test]
    fn oversized_fixture_aggregate_is_rejected() {
        // Codex gate final review round 2: the count cap alone still
        // admits ~102 GiB (MAX_EXPECTED_PAGES × FIXTURE_SIZE_CAP).  The
        // load-bearing aggregate defense is FIXTURE_AGGREGATE_BYTES_CAP.
        // Verify a running-total-exceeded fixture rejects with the
        // dedicated variant.
        //
        // Setup: 3 sparse files at 100 MiB each (well under per-file
        // cap).  Aggregate = 300 MiB > 256 MiB cap → OversizedAggregate
        // on the 3rd file's read.  Sparse `set_len` keeps disk usage
        // trivial; `read_to_end` materializes the zeros (~300 MiB peak
        // in this test's RAM — acceptable for a security regression
        // that would otherwise let an attacker exhaust orders of
        // magnitude more).
        const PAGE_SIZE: u64 = 100 * 1024 * 1024;
        let tmp = tempfile::tempdir().unwrap();
        write_input_html(tmp.path(), b"<!doctype html>");
        let expected = tmp.path().join("expected");
        std::fs::create_dir(&expected).unwrap();
        for n in 0..=2u32 {
            let path = expected.join(format!("page-{n:04}.png"));
            let f = std::fs::File::create(&path).unwrap();
            f.set_len(PAGE_SIZE).unwrap();
            drop(f);
        }
        // Sanity: our 3 files × 100 MiB = 300 MiB > 256 MiB cap.
        // Compile-time to avoid clippy::assertions_on_constants.
        const _: () = assert!(3 * PAGE_SIZE > FIXTURE_AGGREGATE_BYTES_CAP);

        match load_fixture(tmp.path()) {
            Err(FixtureError::OversizedFixtureAggregate {
                fixture_dir,
                total,
                cap,
            }) => {
                assert_eq!(fixture_dir, tmp.path());
                assert_eq!(cap, FIXTURE_AGGREGATE_BYTES_CAP);
                // Total tallied through the (rejected-on-add) 3rd page.
                assert!(
                    total > FIXTURE_AGGREGATE_BYTES_CAP,
                    "total {total} should exceed cap {cap}"
                );
            }
            other => panic!("expected OversizedFixtureAggregate, got {other:?}"),
        }
    }

    /// safe_open must reject a symlink at open time on unix. POSIX mandates
    /// ELOOP (Linux, macOS, modern FreeBSD comply); legacy BSDs (NetBSD,
    /// OpenBSD, FreeBSD <10) return EMLINK or EFTYPE. The test accepts any
    /// Err on unix, because a passing implementation must not follow the
    /// symlink regardless of the exact errno. The Ok arm is the regression
    /// pin — an implementation that drops custom_flags(O_NOFOLLOW) would
    /// silently follow the link and return Ok(file), failing this test.
    #[cfg(unix)]
    #[test]
    fn safe_open_rejects_symlink_at_open_time() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.bin");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target contents")
            .unwrap();
        let link = dir.path().join("link.bin");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match safe_open(&link) {
            Err(_) => {
                // Sanity-check that the path is still a symlink at
                // observation time — proves the Err is due to O_NOFOLLOW
                // (portable across ELOOP / EMLINK / EFTYPE) rather than
                // an unrelated I/O error like permission or NotFound.
                let post = std::fs::symlink_metadata(&link)
                    .expect("symlink still present after safe_open Err");
                assert!(
                    post.file_type().is_symlink(),
                    "link at test observation time was not a symlink"
                );
            }
            Ok(_) => panic!("safe_open followed the symlink (O_NOFOLLOW not applied)"),
        }
    }

    /// End-to-end pin: read_bounded_fixture_file rejects a symlink at the
    /// pre-open `symlink_metadata` check.  This test does NOT exercise the
    /// O_NOFOLLOW path (safe_open never runs because the pre-open check
    /// short-circuits) — that unit is covered by
    /// `safe_open_rejects_symlink_at_open_time`.  Kept to pin the full-path
    /// behavior against future refactors that might reorder the checks.
    #[cfg(unix)]
    #[test]
    fn read_bounded_fixture_file_rejects_symlink_via_pre_open_check() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
        let target = dir.path().join("target.bin");
        std::fs::File::create(&target)
            .unwrap()
            .write_all(b"target")
            .unwrap();
        let link = dir.path().join("link.bin");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        match read_bounded_fixture_file(&link, &canonical_root) {
            Err(FixtureError::SymlinkRejected { path }) => {
                assert_eq!(path, link);
            }
            other => panic!("expected SymlinkRejected, got {other:?}"),
        }
    }

    /// Regression pin: O_NOFOLLOW on the internal open path does not reject a
    /// legitimate regular file.  Without this test, an implementation that
    /// broke the safe_open fallback (e.g. accidentally always returning
    /// ELOOP) would be missed by the symlink-only tests.
    #[test]
    fn read_bounded_fixture_file_accepts_regular_file_with_nofollow() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
        let file_path = dir.path().join("regular.bin");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"regular content")
            .unwrap();

        let bytes = read_bounded_fixture_file(&file_path, &canonical_root)
            .expect("regular file should be accepted");
        assert_eq!(bytes, b"regular content");
    }

    // ------------------------------------------------------------------
    // Post-open fd-based fstat + fd-bound containment tests (bd
    // raikiri-spike-y92o, porting raikiri_dom::fonts's check_open_handle_regular
    // / check_open_handle_containment — bd raikiri-spike-f4j /
    // raikiri-spike-1ef — to this crate's read_bounded_fixture_file)
    // ------------------------------------------------------------------
    //
    // As with the fonts.rs sibling, the actual TOCTOU swap windows these
    // helpers close (regular-file → FIFO/device between pre-open metadata
    // and safe_open; intermediate-dir swap between pre-open canonicalize
    // and safe_open's own pathname re-resolution) are not deterministically
    // reachable through the composed `read_bounded_fixture_file` — they
    // require a second attacker action landing in a microseconds-wide
    // window between two syscalls on the same thread. The defense is
    // instead exercised at the primitive level: `check_open_handle_regular`
    // / `check_open_handle_containment` called directly against a `File`
    // handle that already represents the "swapped" state.
    //
    // `safe_open` sets `O_NONBLOCK` (mirroring fonts.rs, see `safe_open`'s
    // doc), so a FIFO test analogous to fonts.rs's
    // `check_open_handle_regular_rejects_fifo` exists below — see
    // `check_open_handle_regular_rejects_fifo`. The `/dev/null` char-device
    // test exercises the identical `!metadata.file_type().is_file()` branch
    // on a kind that opens immediately even without `O_NONBLOCK`.

    /// `check_open_handle_regular` accepts a regular file opened via
    /// `safe_open` — the primitive-level happy-path regression pin. bd
    /// raikiri-spike-y92o.
    #[test]
    fn check_open_handle_regular_accepts_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regular.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"regular content")
            .unwrap();

        let file = safe_open(&path).expect("safe_open should succeed on regular file");
        check_open_handle_regular(&file, &path).expect("regular file must pass post-open fstat");
    }

    /// `safe_open` + `check_open_handle_regular` rejects `/dev/null`
    /// (character device). `/dev/null` opens immediately even without
    /// `O_NONBLOCK` (no writer-attachment blocking like a FIFO), so this
    /// exercises the fd-based fstat's non-regular-kind rejection without
    /// needing `O_NONBLOCK`. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_regular_rejects_char_device`.
    /// bd raikiri-spike-y92o.
    #[cfg(unix)]
    #[test]
    fn check_open_handle_regular_rejects_char_device() {
        let path = Path::new("/dev/null");
        let file = safe_open(path).expect(
            "safe_open on /dev/null should succeed (regular open semantics on char device)",
        );
        match check_open_handle_regular(&file, path) {
            Err(FixtureError::NotRegularFilePostOpen { path: rejected }) => {
                assert_eq!(rejected, path);
            }
            // cov:ignore: diagnostic-only fallthrough — the expected `Err`
            // arm immediately above is what this test asserts and is
            // pinned by the same test run; this arm exists only to turn an
            // implementation regression into a readable panic message
            // instead of falling through silently.
            other => panic!(
                "expected NotRegularFilePostOpen for /dev/null post-open fstat, got {other:?}"
            ),
        }
    }

    /// `safe_open` + `check_open_handle_regular` rejects a FIFO. Two
    /// asserts pin the composite defense:
    ///
    /// 1. `safe_open` returns `Ok` (without hanging) — proves the
    ///    `O_NONBLOCK` addition (bd raikiri-spike-z719) prevents `open()`
    ///    from blocking on a writer-less FIFO. Without `O_NONBLOCK`,
    ///    `open(O_RDONLY)` on a writer-less FIFO blocks indefinitely at the
    ///    syscall itself and the test would deadlock (never reach the
    ///    fstat).
    /// 2. `check_open_handle_regular` returns
    ///    `Err(FixtureError::NotRegularFilePostOpen)` — proves the fd-based
    ///    `File::metadata()` fstat correctly identifies the FIFO kind and
    ///    would reject before any read from the pipe.
    ///
    /// This crate's pre-open gate in `read_bounded_fixture_file` already
    /// rejects a FIFO placed directly at `input.html` or
    /// `expected/page-*.png` (see `fifo_as_input_html_is_rejected_not_blocked`
    /// / `fifo_at_expected_png_is_rejected_not_blocked`) without ever
    /// calling `safe_open`, so this test calls `safe_open` directly — the
    /// same isolation fonts.rs's sibling test uses — to exercise the
    /// post-open detection path that only fires on a real TOCTOU race in
    /// the composed pipeline. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_regular_rejects_fifo`. bd
    /// raikiri-spike-z719.
    #[cfg(unix)]
    #[test]
    fn check_open_handle_regular_rejects_fifo() {
        let tmp = tempfile::tempdir().unwrap();
        let fifo = tmp.path().join("evil.bin");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo(1) should be available on unix hosts");
        assert!(status.success(), "mkfifo failed for {}", fifo.display());

        // safe_open must return Ok(fd) without blocking — O_NONBLOCK
        // regression guard. A test process reaching this assertion proves
        // `open()` did not hang on the writer-less FIFO.
        let file = safe_open(&fifo).expect(
            "safe_open on writer-less FIFO should return Ok immediately via O_NONBLOCK, \
             not block or Err — if this fails the O_NONBLOCK flag was dropped or the \
             open path regressed",
        );

        match check_open_handle_regular(&file, &fifo) {
            Err(FixtureError::NotRegularFilePostOpen { path }) => {
                assert_eq!(path, fifo);
            }
            // cov:ignore: diagnostic-only fallthrough — the expected `Err`
            // arm immediately above is what this test asserts and is
            // pinned by the same test run; this arm exists only to turn an
            // implementation regression into a readable panic message
            // instead of falling through silently.
            other => {
                panic!("expected NotRegularFilePostOpen for FIFO post-open fstat, got {other:?}")
            }
        }
    }

    /// `check_open_handle_containment` accepts a file whose fd resolves
    /// under the given `canonical_root` — the happy-path regression pin
    /// that the `/proc/self/fd/<fd>` readlink does not spuriously reject
    /// files that are, in fact, inside the root. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_containment_accepts_file_within_root`.
    /// bd raikiri-spike-y92o.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_open_handle_containment_accepts_file_within_root() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
        let path = canonical_root.join("inside.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside content")
            .unwrap();

        let file = safe_open(&path).expect("safe_open should succeed on regular file");
        check_open_handle_containment(&file, &path, &canonical_root)
            .expect("fd resolving inside canonical_root must pass containment recheck");
    }

    /// `check_open_handle_containment` rejects a file whose fd resolves
    /// outside the given `canonical_root`. This is the primitive-level pin
    /// of the actual defense: even though `safe_open` opened the file
    /// successfully (it is a perfectly regular file, just not under the
    /// expected root), the fd-derived `/proc/self/fd/<fd>` path fails
    /// `starts_with(canonical_root)` and the recheck rejects — proving the
    /// check does not trust a path string, only the fd's own resolved
    /// target. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_containment_rejects_file_outside_root`.
    /// bd raikiri-spike-y92o.
    #[cfg(target_os = "linux")]
    #[test]
    fn check_open_handle_containment_rejects_file_outside_root() {
        let fixture_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(fixture_tmp.path()).unwrap();
        let canonical_outside = std::fs::canonicalize(outside_tmp.path()).unwrap();

        let outside_leaf = canonical_outside.join("outside.bin");
        std::fs::File::create(&outside_leaf)
            .unwrap()
            .write_all(b"outside content")
            .unwrap();

        let file = safe_open(&outside_leaf).expect("safe_open should succeed on regular file");
        match check_open_handle_containment(&file, &outside_leaf, &canonical_root) {
            Err(FixtureError::PathEscapePostOpen { canonical, root }) => {
                assert_eq!(canonical, outside_leaf);
                assert_eq!(root, canonical_root);
            }
            // cov:ignore: diagnostic-only fallthrough — the expected `Err`
            // arm immediately above is what this test asserts and is
            // pinned by the same test run; this arm exists only to turn an
            // implementation regression into a readable panic message
            // instead of falling through silently.
            other => panic!(
                "expected PathEscapePostOpen for fd resolving outside canonical_root, got {other:?}"
            ),
        }
    }

    // ------------------------------------------------------------------
    // Apple-platform post-open fd-bound containment tests (bd
    // raikiri-spike-895r — Apple-platform arm of the same defense the
    // Linux tests above pin, via `fcntl(fd, F_GETPATH, ..)` instead of
    // `/proc/self/fd/<fd>` readlink; mirrors
    // `raikiri_dom::fonts`'s sibling tests from bd raikiri-spike-0nww).
    // ------------------------------------------------------------------
    //
    // These mirror `check_open_handle_containment_accepts_file_within_root`
    // / `_rejects_file_outside_root` above structurally (same setup, same
    // assertions) — only the underlying platform primitive differs. They
    // do not run in this repo's Linux CI and have never executed against
    // a real Apple toolchain (no Apple CI target exists here as of
    // 2026-08-10; see the "Untested in CI" section on the
    // `check_open_handle_containment` Apple-platform impl above for the
    // accepted-risk rationale and the `F_GETPATH` path-form caveat to
    // check first if either of these ever fails on real Apple CI).

    /// `check_open_handle_containment` (Apple-platform arm) accepts a file
    /// whose fd resolves under the given `canonical_root` — the happy-path
    /// regression pin that `fcntl(fd, F_GETPATH, ..)` does not spuriously
    /// reject files that are, in fact, inside the root. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_containment_accepts_file_within_root`.
    /// bd raikiri-spike-895r.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos"
    ))]
    #[test]
    fn check_open_handle_containment_apple_accepts_file_within_root() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
        let path = canonical_root.join("inside.bin");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"inside content")
            .unwrap();

        let file = safe_open(&path).expect("safe_open should succeed on regular file");
        check_open_handle_containment(&file, &path, &canonical_root)
            .expect("fd resolving inside canonical_root must pass containment recheck");
    }

    /// `check_open_handle_containment` (Apple-platform arm) rejects a file
    /// whose fd resolves outside the given `canonical_root`. Mirrors the
    /// Linux primitive-level pin: even though `safe_open` opened the file
    /// successfully (it is a perfectly regular file, just not under the
    /// expected root), the fd-derived `F_GETPATH` path fails
    /// `starts_with(canonical_root)` and the recheck rejects — proving the
    /// check does not trust a path string, only the fd's own resolved
    /// target. Mirrors
    /// `raikiri_dom::fonts::check_open_handle_containment_rejects_file_outside_root`.
    /// bd raikiri-spike-895r.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos"
    ))]
    #[test]
    fn check_open_handle_containment_apple_rejects_file_outside_root() {
        let fixture_tmp = tempfile::tempdir().unwrap();
        let outside_tmp = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(fixture_tmp.path()).unwrap();
        let canonical_outside = std::fs::canonicalize(outside_tmp.path()).unwrap();

        let outside_leaf = canonical_outside.join("outside.bin");
        std::fs::File::create(&outside_leaf)
            .unwrap()
            .write_all(b"outside content")
            .unwrap();

        let file = safe_open(&outside_leaf).expect("safe_open should succeed on regular file");
        match check_open_handle_containment(&file, &outside_leaf, &canonical_root) {
            Err(FixtureError::PathEscapePostOpen { canonical, root }) => {
                assert_eq!(canonical, outside_leaf);
                assert_eq!(root, canonical_root);
            }
            // cov:ignore: diagnostic-only fallthrough — the expected `Err`
            // arm immediately above is what this test asserts and is
            // pinned by the same test run; this arm exists only to turn an
            // implementation regression into a readable panic message
            // instead of falling through silently.
            other => panic!(
                "expected PathEscapePostOpen for fd resolving outside canonical_root, got {other:?}"
            ),
        }
    }

    /// **Race regression** (raikiri-spike-51n, follow-up to raikiri-spike-8yu):
    /// races a writer against a reader across the exact TOCTOU window
    /// `safe_open` closes — the pre-open `symlink_metadata` check vs. the
    /// real open — and asserts the reader can never observe the symlink
    /// target's ("evil") contents.
    ///
    /// ## Design
    ///
    /// A writer thread atomically swaps `target` back and forth between a
    /// regular file and a symlink to `evil.bin`, both swaps done via
    /// `rename(2)` of a pre-built source onto `target`. `rename` replaces
    /// the destination in one syscall regardless of the entry's prior
    /// type, so there is no "missing file" window to conflate with the
    /// race under test — unlike the `rm target; symlink(evil, target)` (or
    /// `rm target; File::create(target)`) sequence the issue sketches,
    /// which opens an ENOENT gap that races something else (a plain
    /// not-found error) rather than the metadata-vs-open TOCTOU this test
    /// targets. A reader thread concurrently calls
    /// `read_bounded_fixture_file(target, canonical_root)` in a tight loop.
    ///
    /// `evil.bin` is placed **inside** `canonical_root`, deliberately — if
    /// it lived outside the fixture root, a weakened build (no
    /// `O_NOFOLLOW`) would still get caught by the *unrelated* `PathEscape`
    /// containment check (`canonicalize` follows the symlink before
    /// `safe_open` ever runs), and this test would pass even with the
    /// `O_NOFOLLOW` defense removed — zero detection power. Keeping
    /// `evil.bin` in-root routes execution through `safe_open`, the one
    /// line this test exists to pin. **Do not relocate `evil.bin` outside
    /// the tmp root** — that silently neuters the test.
    ///
    /// ## Assertion
    ///
    /// `Ok(bytes)` must always equal `REGULAR_CONTENTS` exactly;
    /// `Ok(bytes) == EVIL_CONTENTS` is an immediate hard failure — the
    /// property this test exists to guard. Any `Err` is accepted: by
    /// 8yu's design the pre-open `symlink_metadata` rejection and the
    /// open-time `O_NOFOLLOW` rejection both map to the same
    /// `SymlinkRejected` variant (intentionally indistinguishable to the
    /// caller), so this test does not attempt to prove which of the two
    /// gates fired on a given iteration — only that neither ever leaks
    /// `evil.bin`'s contents.
    ///
    /// After the loop, `ok_regular > 0 && symlink_rejected > 0` proves the
    /// writer was actually observed in both states (guards against a
    /// vacuous pass where the writer thread never got scheduled, or the
    /// target never actually changed type). It does **not** prove the
    /// tight metadata-says-regular-then-open-sees-symlink window was hit —
    /// that window is invisible from outside the function under test.
    /// `symlink_rejected` fires overwhelmingly from the cheaper pre-open
    /// check, since the target is a symlink for roughly half of
    /// wall-clock time.
    ///
    /// ## Flakiness / gating
    ///
    /// Timing-dependent by nature (racing OS thread scheduling against a
    /// rename is the whole point), so this is `#[ignore]`d to keep `cargo
    /// test --workspace` deterministic. `#[ignore]` (rather than an
    /// env-var early-return) is deliberate: an env-var gate that
    /// early-returns success on a skipped run reports the test as green
    /// without it ever having run — a silent-pass hazard. `#[ignore]`
    /// reports "ignored" in the default run, which is honest signal that
    /// distinguishes "ran and passed" from "didn't run".
    ///
    /// Run manually:
    ///
    /// ```text
    /// cargo test -p raikiri-vrt --lib -- --ignored concurrent_leaf_swap_never_yields_evil_contents
    /// ```
    ///
    /// `ITERATIONS` was sized empirically: with `safe_open`'s `O_NOFOLLOW`
    /// temporarily reverted to a plain `File::open` (bd raikiri-spike-51n),
    /// 8 repeated runs observed the first `Ok(evil contents)` at iteration
    /// 6, 7, 7, 8, 8, 49, 82, and 171 — worst case 171. `ITERATIONS = 20_000`
    /// here is ~100x that worst-observed margin, so a regression
    /// reintroducing the bug reliably trips this test rather than getting
    /// lucky.
    #[cfg(unix)]
    #[test]
    #[ignore = "timing-dependent race simulation; run manually: \
                cargo test -p raikiri-vrt --lib -- --ignored \
                concurrent_leaf_swap_never_yields_evil_contents"]
    fn concurrent_leaf_swap_never_yields_evil_contents() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        const REGULAR_CONTENTS: &[u8] = b"regular contents";
        const EVIL_CONTENTS: &[u8] = b"evil contents";
        const ITERATIONS: usize = 20_000;

        let dir = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(dir.path()).unwrap();
        let target = dir.path().join("target.bin");
        // Deliberately inside canonical_root — see "Design" above. Moving
        // this outside the root would make the containment check (not
        // O_NOFOLLOW) the thing that rejects the race, neutering the test.
        let evil = dir.path().join("evil.bin");
        std::fs::write(&evil, EVIL_CONTENTS).unwrap();

        // Two swap sources the writer alternately `rename`s onto `target`.
        // Each is recreated after being consumed by a `rename` (rename
        // moves the source; it doesn't copy it).
        let regular_src = dir.path().join("regular_src.bin");
        let evil_link_src = dir.path().join("evil_link_src.bin");
        std::fs::write(&regular_src, REGULAR_CONTENTS).unwrap();
        std::os::unix::fs::symlink(&evil, &evil_link_src).unwrap();
        // Seed `target` so the reader always has something to open.
        std::fs::rename(&regular_src, &target).unwrap();
        std::fs::write(&regular_src, REGULAR_CONTENTS).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&stop);
        let writer_target = target.clone();
        let writer_evil = evil.clone();
        let writer_regular_src = regular_src.clone();
        let writer_evil_link_src = evil_link_src.clone();
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            while !writer_stop.load(Ordering::Relaxed) {
                std::fs::rename(&writer_regular_src, &writer_target)?;
                std::fs::write(&writer_regular_src, REGULAR_CONTENTS)?;

                std::fs::rename(&writer_evil_link_src, &writer_target)?;
                std::os::unix::fs::symlink(&writer_evil, &writer_evil_link_src)?;
            }
            Ok(())
        });

        // The reader loop runs inside `catch_unwind` so that a failing
        // assertion (the exact case this test exists to catch) still lets
        // us signal `stop` and join the writer below, rather than leaking
        // a spinning background thread into the rest of the test binary's
        // process if this test is ever run non-isolated. The original
        // panic (with its diagnostic message) is re-raised via
        // `resume_unwind` afterward — this is bookkeeping around the
        // panic, not suppression of it.
        let loop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut ok_regular = 0usize;
            let mut symlink_rejected = 0usize;
            let mut other_err = 0usize;
            for i in 0..ITERATIONS {
                match read_bounded_fixture_file(&target, &canonical_root) {
                    Ok(bytes) => {
                        assert_ne!(
                            bytes, EVIL_CONTENTS,
                            "SECURITY REGRESSION at iteration {i}: reader observed the \
                             symlink target's contents through the leaf-swap race \
                             (O_NOFOLLOW / pre-open check both failed to close the \
                             TOCTOU window)"
                        );
                        assert_eq!(
                            bytes, REGULAR_CONTENTS,
                            "iteration {i}: reader returned Ok with unexpected contents: {bytes:?}"
                        );
                        ok_regular += 1;
                    }
                    Err(FixtureError::SymlinkRejected { .. }) => symlink_rejected += 1,
                    Err(_) => other_err += 1,
                }
            }
            (ok_regular, symlink_rejected, other_err)
        }));

        stop.store(true, Ordering::Relaxed);
        let writer_result = writer.join();

        let (ok_regular, symlink_rejected, other_err) = match loop_result {
            Ok(counts) => counts,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        writer_result
            .expect("writer thread panicked")
            .expect("writer thread hit an unexpected IO error");

        assert!(
            ok_regular > 0,
            "reader never observed the regular-file state — race not exercised"
        );
        assert!(
            symlink_rejected > 0,
            "reader never observed the symlink state — writer thread may not have \
             been scheduled, or the swap never actually raced the reader"
        );
        eprintln!(
            "concurrent_leaf_swap_never_yields_evil_contents: {ITERATIONS} iterations \
             — ok_regular={ok_regular} symlink_rejected={symlink_rejected} other_err={other_err}"
        );
    }
}
