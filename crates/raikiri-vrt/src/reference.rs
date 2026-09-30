//! raikiri-vrt::reference — reference-fixture test setup for VRT tests.
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

// Threat-model docs cross-link `FIXTURE_SIZE_CAP` / `MAX_EXPECTED_PAGES`
// / `FIXTURE_AGGREGATE_BYTES_CAP` / `read_bounded_fixture_file` (module-private
// items) from public `FixtureError` variants and `load_fixture` docstring, so
// the links resolve under `--document-private-items` but trip `-D warnings` on
// the public build. `publish = false` dev-only crate; keep the cross-links.
#![allow(rustdoc::private_intra_doc_links)]

use std::fmt;
use std::path::{Path, PathBuf};

/// Maximum per-fixture-file size cap (input.html or expected/page-*.png).
///
/// Mirrors `raikiri_dom::fonts::FONT_SIZE_CAP` (100 MiB) as a "large but
/// bounded" fixture size — real reference PNGs are well under 10 MiB and
/// input.html payloads are a few KiB, so 100 MiB leaves ample headroom while
/// still bounding attacker-supplied huge files.
///
/// **Threat surface coverage**:
///
/// - **symlink → sensitive file / `/dev/zero`**: `symlink_metadata` +
///   `is_symlink()` reject on both `input.html` and each
///   `expected/page-*.png`, plus an `expected/`-as-symlink pre-check
/// - **direct char/block device / FIFO placement** (e.g. attacker `mkfifo
///   input.html` or `mkfifo expected/page-0000.png`): `!is_file()` gate
///   rejects non-regular files.  `metadata.len()` is unreliable for devices,
///   so this check is load-bearing and cannot be replaced by the size cap
///   alone (mirrors the same `is_file()` reasoning used elsewhere in this
///   defense family).  The related
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
///   `canonicalize` + `starts_with(canonical_root)` prefix check, plus
///   the post-open fd-derived recheck in
///   `raikiri_traits::io::read_bounded_contained_file` for the residual
///   window between that check and the open
/// - **leaf-swap (TOCTOU) between the pre-open `symlink_metadata` check and
///   the open**: `raikiri_traits::io::open_bounded_regular_file`'s
///   `O_NOFOLLOW` (unix) / reparse-point-aware open (Windows), with an
///   ELOOP-or-`symlink_metadata`-recheck fallback and a post-open fd-based
///   kind recheck for the FIFO/device-swap subclass
/// - **mid-read grow (TOCTOU)**: the `+1-probe` pattern in
///   `raikiri_traits::io::read_bounded_from_open_file` catches files that
///   grow between the `metadata.len()` check and the actual read
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
/// fixtures are single-digit pages, paged-media exports are hundreds)
/// while low enough to reject in tests without a 10 000-file test setup.
const MAX_EXPECTED_PAGES: usize = 1_024;

/// Maximum total bytes across all `expected/page-*.png` entries.
///
/// Load-bearing aggregate-memory defense.  Without this, [`MAX_EXPECTED_PAGES`] alone
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
/// Legitimate fixtures approaching this cap should raise a follow-up request
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
/// Only `EXACT` is currently verified — TIER2/TIER3 fields are shaped for a
/// future platform matrix but the slow path is not currently exercised in tests.
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
    /// Kept as `IoError` (not `Io`) for public API stability;
    /// the sibling workspace convention (`FontError::Io`, `ParseError::Io`)
    /// would prefer bare `Io`, but renaming this variant is a public API
    /// break for `raikiri-vrt` consumers.  The desired rename is tracked as
    /// follow-up work, bundled with the next coordinated API-break window.
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
    /// See [`FIXTURE_SIZE_CAP`] for the full threat model.
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
    /// Post-open, fd-based fstat inside
    /// `raikiri_traits::io::open_bounded_regular_file` found the descriptor
    /// the open returned does not resolve to a regular file. Race-free
    /// complement to [`FixtureError::NotRegularFile`]: the pre-open
    /// path-based `symlink_metadata` check can observe a regular file that
    /// gets swapped for a FIFO/device before the open runs; the traits
    /// helper's post-open check instead consults the inode already bound to
    /// the opened descriptor, so no second path lookup can be raced.
    /// `map_reject_reason` attaches `path` when translating
    /// `raikiri_traits::io::RejectReason::NotRegularFilePostOpen` (which
    /// carries no path of its own) into this variant.
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
    /// Post-open, fd-bound containment recheck (inside
    /// [`raikiri_traits::io::read_bounded_contained_file`]) found the
    /// already-opened descriptor resolves outside `canonical_root`.
    /// Unlike [`FixtureError::PathEscape`] (a fresh, path-based
    /// `canonicalize` re-resolution, run after the open and itself
    /// TOCTOU-vulnerable to a second swap before this recheck runs), this
    /// variant is derived from the fd the
    /// open actually returned — on Linux via `/proc/self/fd/<fd>`, on Apple
    /// platforms via `fcntl(fd, F_GETPATH, ..)` — so firing here means an
    /// intermediate-directory swap happened inside the window between the
    /// open and this fd-based recheck. Mirrors
    /// `raikiri_dom::fonts::FontReadReject::PathEscapePostOpen`.
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
    /// kicks in.
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
                    "fixture entry's opened fd resolves to {} which escapes fixture root {} (TOCTOU-swap between the open and this containment recheck)",
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

    // EXACT optimized path: byte equality skips per-pixel iteration.
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

/// Map a [`raikiri_traits::io::RejectReason`] onto this module's
/// [`FixtureError`] taxonomy, attaching `path` (the traits helper carries no
/// `path` field itself — the caller already has it).
///
/// `RejectReason` and `OversizePhase` are both `#[non_exhaustive]`; a future
/// variant this match has not been updated for falls through to the
/// wildcard arm and is treated as an I/O-shaped hard error — this crate
/// already `?`-propagates every `FixtureError` it produces (no warn+skip
/// path exists here to silently downgrade into), so the wildcard preserves
/// that same fail-closed shape rather than inventing a new one.
fn map_reject_reason(path: &Path, reason: raikiri_traits::io::RejectReason) -> FixtureError {
    use raikiri_traits::io::{OversizePhase, RejectReason};
    match reason {
        RejectReason::Symlink => FixtureError::SymlinkRejected {
            path: path.to_path_buf(),
        },
        RejectReason::NotRegularFile => FixtureError::NotRegularFile {
            path: path.to_path_buf(),
        },
        RejectReason::NotRegularFilePostOpen => FixtureError::NotRegularFilePostOpen {
            path: path.to_path_buf(),
        },
        RejectReason::Oversized {
            size,
            cap,
            phase: OversizePhase::PreOpen | OversizePhase::DuringRead,
        } => FixtureError::OversizedFixture {
            path: path.to_path_buf(),
            size,
            cap,
        },
        RejectReason::PathEscape { canonical, root } => {
            FixtureError::PathEscape { canonical, root }
        }
        RejectReason::PathEscapePostOpen { canonical, root } => {
            FixtureError::PathEscapePostOpen { canonical, root }
        }
        RejectReason::Io(source) => FixtureError::IoError {
            path: path.to_path_buf(),
            source,
        },
        // cov:ignore: unreachable while every current RejectReason variant is
        // matched explicitly above; required for its #[non_exhaustive]
        // contract.
        other => FixtureError::IoError {
            path: path.to_path_buf(),
            source: std::io::Error::other(other.to_string()),
        },
    }
}

/// Read a single fixture-tree file via
/// `raikiri_traits::io::read_bounded_contained_file`: leaf-symlink, kind and
/// [`FIXTURE_SIZE_CAP`] gates, containment under `canonical_root` (including
/// an fd-derived post-open recheck against intermediate-directory swaps),
/// and a bounded read that also catches a file growing during the read.
///
/// Every error is propagated, so on a Linux host without procfs mounted the
/// fd-derived recheck fails closed and fixture loading fails.
///
/// See [`FIXTURE_SIZE_CAP`] for the threat model these layers cover.
fn read_bounded_fixture_file(path: &Path, canonical_root: &Path) -> Result<Vec<u8>, FixtureError> {
    raikiri_traits::io::read_bounded_contained_file(path, canonical_root, FIXTURE_SIZE_CAP)
        .map_err(|reason| map_reject_reason(path, reason))
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
/// defense stack against untrusted fixture trees.
/// Briefly: fixture-root-symlink reject, leaf-symlink reject, `!is_file()`
/// reject, per-file 100 MiB size cap, [`MAX_EXPECTED_PAGES`] enumeration
/// count cap, [`FIXTURE_AGGREGATE_BYTES_CAP`] aggregate bytes cap,
/// canonical `page-{N:04}.png` name filter, canonicalized-prefix
/// containment check, and a bounded read that catches TOCTOU-grow.  Symlinks anywhere in the fixture tree — including
/// the fixture-directory anchor itself — are refused even when they'd
/// resolve inside the intended root; this is a deliberate blanket policy
/// (no real reference fixture currently uses symlinks) and tests check the
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
                    continue; // cov:ignore: 4 ASCII digits (0000-9999) always parse to u32; else unreachable
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
/// the env-var mechanism is the current implementation (see spec §13 drift).
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
mod tests;
