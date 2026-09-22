//! CSS Break Table parity coverage is deferred.
//!
//! The exact Fulgur v0.40.0 sweep currently has no PASS table candidates.
//! Keep the measured table cases out of the parity gate until that changes.

/// Table fragmentation remains deferred until a Fulgur v0.40.0 PASS exists.
#[test]
#[ignore = "deferred: Fulgur v0.40.0 has no exact CSS Break Table PASS candidate"]
fn css_break_table_pairs_are_pixel_exact_at_800x600() {}
