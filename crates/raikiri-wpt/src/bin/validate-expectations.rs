//! Validate `expectations/*.txt` files (spec §12.10).
//!
//! Detects Malformed / Duplicate / Conflicting (CI-blocking) and Expired
//! (warning) issues. Runs in CI on every PR via the m1.12 GitHub Actions
//! workflow.
//!
//! Exit codes:
//! - `0`: clean, or only [`raikiri_wpt::lint::Category::Expired`] warnings
//! - `2`: any Malformed / Duplicate / Conflicting failure

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::lint;
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
    let report = lint::run(&dir, now);
    print!("{}", report.format_human());
    if report.has_failures() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
