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
use std::path::PathBuf;

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
    pub const EXACT: Self = Self { max_delta: 0, max_diff_fraction: 0.0 };
    /// Tier 2 tolerance (Linux aarch64, macOS): max_delta=1, max_diff=0.1%.
    pub const TIER2: Self = Self { max_delta: 1, max_diff_fraction: 0.001 };
    /// Tier 3 tolerance (Windows): max_delta=2, max_diff=0.5%.
    pub const TIER3: Self = Self { max_delta: 2, max_diff_fraction: 0.005 };
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
    /// A PNG file under `expected/` failed to decode.
    PngDecodeError {
        /// Path to the PNG file that failed to decode.
        path: PathBuf,
        /// Underlying PNG decoding error.
        source: png::DecodingError,
    },
    /// A rendered page's dimensions did not match the expected golden image.
    DimensionMismatch {
        /// Index of the page with mismatched dimensions.
        page_index: usize,
        /// Expected (width, height).
        expected_wh: (u32, u32),
        /// Actual (width, height).
        actual_wh: (u32, u32),
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
            Self::PngDecodeError { path, source } => {
                write!(f, "PNG decode failed for {}: {source}", path.display())
            }
            Self::DimensionMismatch { page_index, expected_wh, actual_wh } => {
                write!(
                    f,
                    "page {page_index} dimension mismatch: expected {}x{}, actual {}x{}",
                    expected_wh.0, expected_wh.1, actual_wh.0, actual_wh.1,
                )
            }
        }
    }
}

impl std::error::Error for FixtureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError { source, .. } => Some(source),
            Self::PngDecodeError { source, .. } => Some(source),
            _ => None,
        }
    }
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
                x: 10, y: 20,
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
