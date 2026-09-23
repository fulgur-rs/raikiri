//! CSS Text line-break parity coverage is deferred.
//!
//! `line-break-anywhere-001` fails in both Raikiri and Fulgur v0.40.0 at
//! 800x600. Keep it out of the parity gate until a Fulgur PASS exists.

/// `line-break: anywhere` remains deferred until a Fulgur v0.40.0 PASS exists.
#[test]
#[ignore = "deferred: Fulgur v0.40.0 has no exact line-break-anywhere-001 PASS"]
fn line_break_anywhere_is_pixel_exact_at_800x600() {}
