//! CSS Break Grid parity coverage is deferred.
//!
//! The exact Fulgur v0.40.0 sweep currently has no PASS grid candidates.
//! Keep the measured grid cases out of the parity gate until that changes.

/// Grid fragmentation remains deferred until a Fulgur v0.40.0 PASS exists.
#[test]
#[ignore = "deferred: Fulgur v0.40.0 has no exact CSS Break Grid PASS candidate"]
fn css_break_grid_pairs_are_pixel_exact_at_800x600() {}
