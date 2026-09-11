//! Generate WPT CSS dashboard HTML (raikiri version of blitz.is/wpt/css).
//!
//! Usage: cargo run -p raikiri-wpt --bin generate-dashboard -- --output docs/wpt-dashboard.html

use std::path::PathBuf;

fn main() {
    // Simple: always write to docs/wpt-dashboard.html, allow override via second arg
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().position(|a| a == "--output").and_then(|i| args.get(i+1).map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("docs/wpt-dashboard.html"));
    // For now, the dashboard is static (see docs/wpt-dashboard.html). This bin exists as a placeholder
    // for future dynamic generation that runs `cargo test -p raikiri-wpt --test basic_reftests -- --format=json`
    // and aggregates per-area pass rates.
    println!("Dashboard is static at docs/wpt-dashboard.html. Run `cargo test -p raikiri-wpt --test basic_reftests` to update data, then re-run this bin when dynamic generation is implemented.");
    println!("Output requested: {}", output.display());
    // Ensure the static file exists
    if !output.exists() {
        eprintln!("Warning: {} does not exist. Copying from docs/wpt-dashboard.html if available.", output.display());
    }
}
