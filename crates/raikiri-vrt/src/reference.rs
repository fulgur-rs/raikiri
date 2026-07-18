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

use std::fmt;
use std::path::{Path, PathBuf};

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

/// Load a `tests/reference/<name>/` fixture directory.
///
/// Reads `input.html` (required) and any `expected/page-{N:04}.png` files.
/// Pages must be numbered contiguously from 0; gaps produce
/// `FixtureError::NonContiguousPages`. Missing `expected/` (or an empty one)
/// is permitted — this is the update-goldens starting state.
///
/// PNG bytes are stored raw; decoding is deferred until `compare_png` runs.
pub fn load_fixture(fixture_dir: &Path) -> Result<Fixture, FixtureError> {
    let input_path = fixture_dir.join("input.html");
    let input_html = match std::fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureError::MissingInputHtml {
                fixture_dir: fixture_dir.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(FixtureError::IoError {
                path: input_path,
                source,
            });
        }
    };

    let expected_dir = fixture_dir.join("expected");
    let mut numbered: Vec<(u32, Vec<u8>)> = Vec::new();
    if expected_dir.is_dir() {
        let entries = std::fs::read_dir(&expected_dir).map_err(|source| FixtureError::IoError {
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
            // Match "page-XXXX.png" exactly; ignore README.md and other files.
            let Some(stem) = name.strip_suffix(".png") else {
                continue;
            };
            let Some(num_str) = stem.strip_prefix("page-") else {
                continue;
            };
            let Ok(n) = num_str.parse::<u32>() else {
                continue;
            };
            let bytes = std::fs::read(&path).map_err(|source| FixtureError::IoError {
                path: path.clone(),
                source,
            })?;
            numbered.push((n, bytes));
        }
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
}
