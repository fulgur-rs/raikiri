//! Validate `expectations/*.txt` files (spec §12.10).
//!
//! Detects Malformed / Duplicate / Conflicting (CI-blocking) and Expired
//! (warning) issues. Runs in CI on every PR via the GitHub Actions
//! workflow.
//!
//! Exit codes:
//! - `0`: clean, or only [`raikiri_wpt::lint::Category::Expired`] warnings
//! - `2`: any Malformed / Duplicate / Conflicting failure
//! - `3`: `RAIKIRI_WPT_MATRIX` env var is set but did not parse
//!
//! Env vars:
//! - `RAIKIRI_WPT_EXPECTATIONS_DIR` — override the `expectations/` root
//! - `RAIKIRI_WPT_MATRIX` — semicolon-delimited concrete matrix rows
//!   used to suppress spurious `baseline ∩ quarantine`
//!   [`raikiri_wpt::lint::Category::Conflicting`] issues per spec §12.10
//!   filter-overlap analysis. Format:
//!   `platform,arch,renderer,tolerance;platform,arch,renderer,tolerance`.
//!   Missing → conservative default (every overlap is a conflict).

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::lint::{self, RunMatrix};
use time::OffsetDateTime;

fn expectations_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("RAIKIRI_WPT_EXPECTATIONS_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("expectations")
}

fn main() -> ExitCode {
    let dir = expectations_dir();
    let now = OffsetDateTime::now_utc().date();
    let matrix = match std::env::var("RAIKIRI_WPT_MATRIX") {
        Ok(v) => match v.parse::<RunMatrix>() {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("RAIKIRI_WPT_MATRIX parse error: {e}");
                return ExitCode::from(3);
            }
        },
        Err(_) => None,
    };
    let report = lint::run_with_matrix(&dir, now, matrix.as_ref());
    print!("{}", report.format_human());
    if report.has_failures() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
