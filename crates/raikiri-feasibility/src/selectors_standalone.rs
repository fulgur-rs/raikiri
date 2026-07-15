//! selectors standalone (no stylo) — nzv-spike for raikiri-spike-nzv.8
//!
//! # Goal
//!
//! Verify that `selectors` 0.39 can be driven by `cssparser` 0.37 to parse a
//! `SelectorList<Impl>` *without* pulling in `stylo` (memory:
//! `raikiri-implementation-independence`).
//!
//! # Finding (OK — resolved via encapsulation in raikiri-style)
//!
//! During the initial spike we discovered that `selectors::SelectorImpl` bounds
//! `Identifier` / `LocalName` / `NamespaceUrl` on
//! `precomputed_hash::PrecomputedHash`, and that `precomputed-hash` is a
//! transitive dep of `selectors` but not re-exported. The fix is to add
//! `precomputed-hash` as a **direct** dep of the crate that owns the
//! `SelectorImpl`, and to `impl PrecomputedHash for Atom` there.
//!
//! In raikiri's architecture the stylo-equivalent crate is `raikiri-style`, so
//! `precomputed-hash` is encapsulated as a direct dep of that crate only —
//! **not** promoted to `[workspace.dependencies]`. This spike therefore now
//! calls `raikiri_style::parse_selector_list(".btn:hover")` to prove the
//! stylo-free path works end-to-end.
//!
//! # What this file demonstrates
//!
//! * (compile-time) `raikiri_style::RaikiriSelectorImpl` satisfies every
//!   `selectors::SelectorImpl` associated-type / trait bound with **zero**
//!   `stylo` in the dep chain of `raikiri-style` itself.
//! * (run-time) `raikiri_style::parse_selector_list(".btn:hover")` returns
//!   `Ok(SelectorList<RaikiriSelectorImpl>)` and the result round-trips through
//!   `ToCss` back to `.btn:hover`.
//! * (run-time) A multi-selector list (`"a:hover, .btn:active"`) parses
//!   successfully, exercising the `,` combinator path.

use cssparser::ToCss;
use raikiri_style::parse_selector_list;

/// Public helper: parse `input` via raikiri-style's SelectorImpl and
/// stringify errors. Thin wrapper so nzv.12 (feasibility-report) has a
/// stable spike entry-point.
pub fn parse_via_raikiri_style(input: &str) -> Result<String, String> {
    let list = parse_selector_list(input)?;
    let mut out = String::new();
    list.to_css(&mut out)
        .map_err(|e| format!("serialize error: {e:?}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btn_hover_roundtrips_via_raikiri_style() {
        let out = parse_via_raikiri_style(".btn:hover").expect("parse .btn:hover");
        assert_eq!(out, ".btn:hover");
    }

    #[test]
    fn multi_selector_list_parses() {
        let out = parse_via_raikiri_style("a:hover, .btn:active").expect("parse list");
        assert!(out.contains(":hover"));
        assert!(out.contains(":active"));
        assert!(out.contains(','));
    }

    #[test]
    fn unknown_pseudo_class_is_reported_as_error() {
        // ":unsupported" is not one of raikiri-style's M0 seed pseudo-classes
        // (only :hover / :active). Confirms error propagation works.
        let err = parse_via_raikiri_style(":unsupported").expect_err("should error");
        assert!(err.contains("parse error"));
    }
}
