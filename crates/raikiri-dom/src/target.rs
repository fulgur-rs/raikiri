//! GCPM `target-*()` / `element()` runtime resolve — registry data structure.
//!
//! **Ownership** (design §7.0): raikiri-dom (runtime side) owns the resolve
//! state; raikiri-style (static side) emits the placeholders via
//! [`raikiri_traits::GcpmDirective`]. This file lands the runtime working
//! registry only; the resolve pass and the traits-side directive population
//! are deferred children (bd raikiri-spike-96u.2 / .3 / .4).
//!
//! **Canonical shape** (design §7.2, lines 1961-1964):
//! ```text
//! pub struct TargetRegistry {
//!     resolved: HashMap<Symbol, TargetInfo>,
//!     pending_slots: Vec<TargetSlot>,
//! }
//! ```
//!
//! **Divergence from canonical shape** — the design doc writes `pub struct`;
//! this file writes `pub(crate)` and does NOT re-export through the crate
//! root (`lib.rs`). Rationale: the leaf task (bd raikiri-spike-96u.1) scopes
//! the skeleton to raikiri-dom-internal until the eventual traits-side
//! promotion. The field layout — `resolved` / `pending_slots`, `Symbol` key,
//! `TargetInfo` / `TargetSlot` value types — matches §7.2 verbatim.
//!
//! **Name collision note** — [`raikiri_traits::TargetRegistry`]
//! (`crates/raikiri-traits/src/page.rs`) is a distinct public consumer-facing
//! placeholder (M1.1 opaque `#[non_exhaustive]` shell used by
//! `RenderSummary.target_registry`). This dom-internal type is the runtime
//! working state; the two are intentionally separate. Reconcile is
//! out-of-scope of bd raikiri-spike-96u.4 (which scopes to `GcpmDirective`
//! / `ContentValueItem` populate) — tracked as bd raikiri-spike-bsi
//! (blocked on 96u.2 + 96u.3 landing real fields, Sprint 14+).

use std::collections::HashMap;

use raikiri_traits::Symbol;

/// Runtime registry for GCPM `target-*()` / `element()` fragment references.
///
/// A single `TargetRegistry` per document collects (a) fragments that have
/// been walked and can answer target-* queries directly and (b) pending
/// content-value sites that referenced a fragment before it was walked and
/// must be resolved on a second pass.
///
/// Design doc: `docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
/// §7.2. Populated by bd raikiri-spike-96u.2 / .3.
#[derive(Debug, Default)]
#[allow(
    dead_code,
    reason = "GCPM target-* runtime skeleton: fields consumed by bd \
              raikiri-spike-96u.2 (resolve pass) and 96u.3 (pending-slot \
              flush). 96u.1 lands the shape in isolation before runtime \
              wiring."
)]
pub(crate) struct TargetRegistry {
    /// Fragment identifier → resolved target metadata (page index, counter
    /// snapshot, textual content parts). Populated as the runtime walk
    /// encounters `RegisterTarget { fragment_id }` directives
    /// ([`raikiri_traits::GcpmDirective`]).
    resolved: HashMap<Symbol, TargetInfo>,
    /// Content-value sites (`target-counter(...)`, `target-text(...)`,
    /// `element(name)`) that referenced a fragment before it was resolved.
    /// Flushed on a second pass once the registry is fully populated.
    pending_slots: Vec<TargetSlot>,
}

/// Resolved target metadata — populated by bd raikiri-spike-96u.2.
///
/// Placeholder unit struct; the runtime resolve pass will grow this into
/// the target-* answer shape (page index, counter snapshot, text run
/// content-parts) at that point.
#[derive(Debug, Default)]
#[allow(
    dead_code,
    reason = "Placeholder for bd raikiri-spike-96u.2 (target-* resolve pass)."
)]
pub(crate) struct TargetInfo;

/// Unresolved content-value site — populated by bd raikiri-spike-96u.3.
///
/// Placeholder unit struct; the pending-slot flush pass will grow this to
/// carry back-reference data (referring fragment id + the ContentValueItem
/// to fill in) at that point.
#[derive(Debug, Default)]
#[allow(
    dead_code,
    reason = "Placeholder for bd raikiri-spike-96u.3 (pending-slot flush)."
)]
pub(crate) struct TargetSlot;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_registry_default_is_empty() {
        // Constructibility pin: `TargetRegistry::default()` yields an empty
        // registry. 96u.2 (resolve pass) will consume this constructor.
        let reg = TargetRegistry::default();
        assert!(reg.resolved.is_empty());
        assert!(reg.pending_slots.is_empty());
    }

    #[test]
    fn target_registry_shape_matches_design_7_2() {
        // Canonical shape pin (design §7.2, lines 1961-1964):
        //   resolved: HashMap<Symbol, TargetInfo>
        //   pending_slots: Vec<TargetSlot>
        // 96u.2 / .3 will grow TargetInfo / TargetSlot; if this test breaks,
        // the leaf skeleton has drifted from the design and the coordinator
        // should reconcile before landing the follow-up children.
        let mut reg = TargetRegistry::default();
        reg.resolved.insert(Symbol::new("fragment-1"), TargetInfo);
        reg.pending_slots.push(TargetSlot);
        assert_eq!(reg.resolved.len(), 1);
        assert_eq!(reg.pending_slots.len(), 1);
        assert!(reg.resolved.contains_key(&Symbol::new("fragment-1")));
    }
}
