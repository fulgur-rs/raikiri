//! GCPM `target-*()` / `element()` runtime resolve — canonical registry.
//!
//! **Ownership** (design §7.0 line 1904 "shared types → raikiri-traits"): this
//! is the single canonical [`TargetRegistry`] shared across raikiri-dom,
//! raikiri-paint, and consumer crates. raikiri-traits owns the type; the
//! producer side (register-site directive walker,
//! feeding [`raikiri_traits::GcpmDirective::RegisterTarget`]) lives in
//! raikiri-dom and calls [`TargetRegistry::register`] against this
//! canonical instance.
//!
//! **History** — the impl and tests here were previously a `pub(crate)` shadow
//! at `crates/raikiri-dom/src/target.rs` alongside an empty
//! `#[non_exhaustive]` placeholder at `crates/raikiri-traits/src/page.rs`.
//! These were merged: the dom-side shadow was deleted, the field layout /
//! method surface was promoted from `pub(crate)` to `pub`, and this file
//! now hosts the canonical type.
//!
//! **Canonical shape** (design §7.2 lines 1961-1964):
//! ```text
//! pub struct TargetRegistry {
//!     resolved: HashMap<Symbol, TargetInfo>,
//!     pending_slots: Vec<TargetSlot>,
//! }
//! ```
//! `next_sequence: u32` and `page_index: u32` are additive internal-only
//! fields; §7.2 does not enumerate either. Together they stamp every
//! pending slot with its stable handle,
//! [`TargetSlotId`] = `(page_index, sequence)` (design §7.4 line 2098 /
//! §7.6 lines 2312-2317, Finding #4, paired with the
//! `PageContext::page_index` field).
//! `page_index` defaults to 0 and advances via
//! [`TargetRegistry::begin_page`], which also resets `next_sequence` to 0 —
//! design §7.6 "Slot ID の安定性保証" states `sequence` is "page 内
//! 0-indexed" (page-local, not document-wide), so a page-boundary call must
//! restart the local count. Calling contract: `begin_page` once per page,
//! with a monotonically non-decreasing `page_index` (same-index calls are a
//! no-op; a redundant same-page call is fine, a backward call is not),
//! before the first `resolve_target_*` dispatch for that page — enforced at
//! runtime by an `assert!` in [`TargetRegistry::begin_page`] since a
//! backward `page_index` would silently mint a duplicate [`TargetSlotId`]
//! for a still-pending slot from the earlier visit to that page.

use std::collections::HashMap;

use raikiri_style::property::{ContentComponent, ContentPart, CounterStyle};

use crate::dom::Symbol;
use crate::error::TargetSlotId;

/// Runtime registry for GCPM `target-*()` / `element()` fragment references.
///
/// A single `TargetRegistry` per document collects (a) fragments that have
/// been walked and can answer `target-counter` / `target-counters` /
/// `target-text` queries directly and (b) pending content-value sites that
/// referenced a fragment before it was walked and must be resolved on a
/// second pass ([`TargetRegistry::flush_pending`]).
///
/// Producer / register-site walker lives in
/// raikiri-dom; the canonical type (shape + resolve strategy) lives here.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetRegistry {
    /// Fragment identifier → resolved target metadata (counter snapshot,
    /// textual content parts). Populated as the runtime walk encounters
    /// [`crate::GcpmDirective::RegisterTarget`] directives (populator: the
    /// register-site walker in raikiri-dom).
    resolved: HashMap<Symbol, TargetInfo>,
    /// Content-value sites (`target-counter(...)`, `target-counters(...)`,
    /// `target-text(...)`, and future `element(name)`) that referenced a
    /// fragment before it was resolved. Flushed on a second pass via
    /// [`TargetRegistry::flush_pending`] once the registry is fully
    /// populated. Slots for fragments that never get registered remain in
    /// the queue after a flush (streaming intent: a later batch or
    /// finish_render pass may still land them; see design §7.4).
    pending_slots: Vec<TargetSlot>,
    /// Page-local monotonic sequence stamp for pending slots, reset to 0 by
    /// [`Self::begin_page`]. Not in the §7.2 shape — see module-level
    /// "canonical shape" note.
    next_sequence: u32,
    /// 0-based index of the page whose target-* dispatches are currently
    /// being recorded, advanced via [`Self::begin_page`]. Not in the §7.2
    /// shape — see module-level "canonical shape" note. Defaults to 0
    /// (correct for a single-page document; a caller that never advances it
    /// tags every slot to page 0, which is harmless — just uninformative —
    /// until a real per-page driver exists, same gap
    /// `PageContext::begin_page`'s doc flags).
    page_index: u32,
}

/// Resolved target metadata — the counter snapshot and text parts captured
/// when a `RegisterTarget` directive walks the fragment's element.
///
/// **Counter shape**: each named counter carries its full nested stack
/// (`Vec<i32>`, outermost scope first, leaf scope last). `target-counter`
/// reads only the leaf (last element); `target-counters` joins every level
/// with the caller-supplied separator (CSS Content 3 §2.6.2, `counters()`
/// hierarchical join).
///
/// **Divergence from design §11.2** — §11.2's `TargetDefinition` uses
/// `counter_snapshot: HashMap<Symbol, i32>` (a flat single-value snapshot).
/// That shape is insufficient for `target-counters(url, name, sep)`: a
/// nested scope like `[1, 1]` cannot be recovered from a single `i32` and
/// the hierarchical join `"1.1"` is literally unrepresentable. This module
/// therefore carries the full stack per counter; when the §11.2
/// `TargetDefinition` public shape is reconciled with the runtime side, the
/// reconciliation must widen `counter_snapshot` to a stack — and the
/// register-site directive walker must populate the stack rather than the
/// leaf value.
///
/// **Text parts**: keyed by the same
/// [`raikiri_style::property::ContentPart`] variants that
/// `target-text(url, part)` accepts. Missing parts resolve to an empty
/// string (CSS Content 3 §2.6.3 defers text extraction — populating each
/// entry is the responsibility of the register site).
///
/// **Storage note**: `text_parts` is a `Vec<(ContentPart, String)>`, not a
/// `HashMap`, because [`ContentPart`] does not implement `Hash`
/// (raikiri-style keeps it `#[derive(PartialEq, Eq)]` only) and adding
/// `Hash` on the raikiri-style side would be a cross-crate public-surface
/// change out of scope here. The variant set is bounded (4 keywords plus
/// non_exhaustive room), so linear scan on lookup is fine.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetInfo {
    /// counter name → nested stack (outermost first). Empty stack = 0.
    pub counters: HashMap<Symbol, Vec<i32>>,
    /// Text extracted from the register-site element, keyed by
    /// `target-text`'s second argument. Missing keys resolve to `""`.
    /// Linear scan on lookup (bounded 4-variant enum key); see type-level
    /// storage note.
    pub text_parts: Vec<(ContentPart, String)>,
}

impl TargetInfo {
    /// Construct an empty `TargetInfo` (canonical zero-arg constructor per the
    /// [`crate::page`] module contract — input type consumed by
    /// [`TargetRegistry::register`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a text part, returning the empty string when absent
    /// (`target-text` fallback per CSS Content 3 §2.6.3).
    fn text_part(&self, part: ContentPart) -> &str {
        self.text_parts
            .iter()
            .find_map(|(p, s)| (*p == part).then_some(s.as_str()))
            .unwrap_or("")
    }

    /// Single-entry evaluator for a [`TargetRequest`] against this snapshot.
    ///
    /// This helper is the sole request-evaluation site, called from both the
    /// immediate-resolve path ([`TargetRegistry::dispatch`]) and the
    /// second-pass flush ([`TargetRegistry::flush_pending`]). Keeping the
    /// per-variant semantics — missing-counter fallback, empty-stack `"0"`
    /// (CSS Content 3 §2.6.2), absent text-part `""` (§2.6.3) — in one place
    /// prevents the two paths from diverging.
    fn resolve(&self, request: &TargetRequest) -> String {
        match request {
            TargetRequest::Counter { name, style } => {
                // Absent counter → 0 (CSS Content 3 §2.6.2 default-init: an
                // undefined counter has value 0).
                let value = self
                    .counters
                    .get(name)
                    .and_then(|stack| stack.last().copied())
                    .unwrap_or(0);
                format_counter(value, style)
            }
            TargetRequest::Counters {
                name,
                separator,
                style,
            } => {
                // Absent counter *or* an empty stack must render as the
                // single formatted `0`, not an empty string. CSS Content 3
                // §2.6.2: `counters(name)` of an undefined / freshly-reset
                // counter yields the initial value `0` — the join-with-`sep`
                // of a one-element `[0]` is just `"0"`.
                let stack: &[i32] = self.counters.get(name).map(|v| v.as_slice()).unwrap_or(&[]);
                if stack.is_empty() {
                    format_counter(0, style)
                } else {
                    join_counter_stack(stack, separator, style)
                }
            }
            TargetRequest::Text { part } => self.text_part(*part).to_owned(),
        }
    }
}

/// The three `target-*` request kinds this pass resolves.
///
/// Kept crate-private inside raikiri-traits: this enum is a stored form used
/// only inside [`TargetSlot`] / [`TargetRegistry::dispatch`], never in a
/// public signature. The public-facing counterpart is
/// [`crate::strategy::TargetRequest`] (the lifetimed query passed to
/// [`crate::TargetResolver`]) — keeping this internal avoids the name
/// collision at the crate root.
///
/// Mirrors design §11.2's `TargetKind` shape (subset — `Page` is out of
/// current scope; it lands with the `target-page` / `element(name)`
/// follow-up).
#[derive(Debug, Clone)]
pub(crate) enum TargetRequest {
    Counter {
        name: Symbol,
        style: CounterStyle,
    },
    Counters {
        name: Symbol,
        separator: String,
        style: CounterStyle,
    },
    Text {
        part: ContentPart,
    },
}

/// Unresolved content-value site — a forward reference to a fragment that
/// hasn't been walked yet. Crate-private: only observed externally through
/// [`ResolveOutcome::Pending`]'s [`TargetSlotId`] handle and later
/// [`PendingResolution::slot_id`].
///
/// **Stable handle = `TargetSlotId = (page_index, sequence)`** (design §7.4
/// line 2098 / §7.6 lines 2312-2317, Finding #4 — not §11.2, whose
/// `TargetSlot` is a distinct, not-yet-built public paint-layer type with a
/// different shape `{ id, fragment_id, kind, rect, resolved, fallback_text
/// }`): pairing the sequence with `page_index` keeps (a) slot ids
/// byte-identical across streaming/batch iterations and (b) lets sinks
/// address a slot by its owning page.
/// `page_index` comes from [`TargetRegistry::begin_page`] (paired with the
/// `PageContext::page_index` field) — see the module doc's "Canonical shape"
/// note for the page-local `sequence` reset this implies.
#[derive(Debug, Clone)]
pub(crate) struct TargetSlot {
    pub(crate) id: TargetSlotId,
    pub(crate) fragment_id: Symbol,
    pub(crate) request: TargetRequest,
}

/// Outcome of a `resolve_target_*` call.
///
/// `Resolved(String)` — the fragment was in `resolved` (or the URL was a
/// non-fragment fallback), the formatted answer is returned inline.
///
/// `Pending(slot_id)` — the fragment is not yet in `resolved`; a pending
/// slot addressed by this [`TargetSlotId`] has been queued in the registry.
/// Call [`TargetRegistry::flush_pending`] after further register calls to
/// receive the eventual value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolveOutcome {
    /// Immediately resolved — the fragment was in `resolved` or the URL was
    /// a non-fragment fallback (in which case the payload is empty).
    Resolved(String),
    /// Deferred — the [`TargetSlotId`] addresses the queued pending slot;
    /// [`TargetRegistry::flush_pending`] will return the resolved value
    /// once the fragment lands.
    Pending(TargetSlotId),
}

/// One entry returned by [`TargetRegistry::flush_pending`].
///
/// `value = Some(...)` means the slot resolved on this flush; `None` is
/// unreachable in the current implementation (unresolved slots are retained
/// in the pending queue instead of being flushed with `None`), but the field
/// is `Option<String>` to keep the future "give up after N passes" path
/// backwards-compatible.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PendingResolution {
    /// [`TargetSlotId`] of the slot that was resolved on this flush —
    /// matches the value returned by the original
    /// [`ResolveOutcome::Pending`].
    pub slot_id: TargetSlotId,
    /// Resolved value; `Some(...)` on successful resolve. Reserved `None`
    /// for the future "give up after N passes" streaming exit — see the
    /// type-level note.
    pub value: Option<String>,
}

impl TargetRegistry {
    /// Construct an empty `TargetRegistry` (canonical zero-arg constructor,
    /// stable across the promotion from the earlier opaque placeholder).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a resolved target — called from the register-site directive
    /// walker once an element with an `id` attribute is fully seen.
    ///
    /// **First-wins on duplicate fragment id.** Uses `entry().or_insert` so
    /// a second `register(id, info)` for the same fragment is a no-op. This
    /// matches the DOM id-resolution model: `getElementById` and every
    /// spec'd id-based lookup resolves to the **first element in tree
    /// order** when multiple elements share an id (which is invalid HTML in
    /// the first place — HTML §3.2.6.1 "id attribute" — but the resolution
    /// behavior is still well-defined). The register-site walker is
    /// expected to invoke `register()` in tree order, so `or_insert`
    /// preserves the spec's first-in-tree-order
    /// semantics without the walker having to check for duplicates.
    pub fn register(&mut self, fragment_id: Symbol, info: TargetInfo) {
        self.resolved.entry(fragment_id).or_insert(info);
    }

    /// Resolve `target-counter(url, name, style)`.
    ///
    /// URL parsing, resolved/pending dispatch, and per-variant evaluation
    /// are shared with the other `target-*` entrypoints via the private
    /// `dispatch` helper. Absent counter yields `"0"` (CSS Content 3
    /// §2.6.2). Non-fragment URLs resolve to an empty string (raikiri is
    /// per-document, external target refs are unresolvable).
    pub fn resolve_target_counter(
        &mut self,
        url: &str,
        name: Symbol,
        style: CounterStyle,
    ) -> ResolveOutcome {
        self.dispatch(url, TargetRequest::Counter { name, style })
    }

    /// Resolve `target-counters(url, name, separator, style)` — hierarchical
    /// counter join (CSS Content 3 §2.6.2, `counters()` join semantics).
    ///
    /// Every element of the counter stack is formatted per `style` and
    /// joined with `separator`. An absent (or empty-stack) counter yields
    /// `"0"`, not `""` — an undefined counter's initial value is `0` per
    /// §2.6.2, so `counters(name, sep)` returns the single formatted `0`.
    pub fn resolve_target_counters(
        &mut self,
        url: &str,
        name: Symbol,
        separator: &str,
        style: CounterStyle,
    ) -> ResolveOutcome {
        self.dispatch(
            url,
            TargetRequest::Counters {
                name,
                separator: separator.to_owned(),
                style,
            },
        )
    }

    /// Resolve `target-text(url, part)` — text extraction (CSS Content 3
    /// §2.6.3). Missing parts resolve to `""`.
    pub fn resolve_target_text(&mut self, url: &str, part: ContentPart) -> ResolveOutcome {
        self.dispatch(url, TargetRequest::Text { part })
    }

    /// Single-entry dispatch for the three `target-*` entrypoints.
    ///
    /// Parses the fragment, looks up `resolved`, and either returns
    /// [`ResolveOutcome::Resolved`] (via [`TargetInfo::resolve`]) or queues
    /// a [`TargetSlot`] and returns [`ResolveOutcome::Pending`]. Non-fragment
    /// URLs short-circuit to an empty string without touching
    /// `pending_slots` (leaked pending slots would grow unbounded on
    /// external URL references).
    fn dispatch(&mut self, url: &str, request: TargetRequest) -> ResolveOutcome {
        let Some(fragment) = parse_fragment(url) else {
            return ResolveOutcome::Resolved(String::new());
        };
        let key = Symbol::new(fragment);
        if let Some(info) = self.resolved.get(&key) {
            ResolveOutcome::Resolved(info.resolve(&request))
        } else {
            let id = TargetSlotId {
                page_index: self.page_index,
                sequence: self.next_sequence(),
            };
            self.pending_slots.push(TargetSlot {
                id,
                fragment_id: key,
                request,
            });
            ResolveOutcome::Pending(id)
        }
    }

    /// Drain the pending queue, resolving every slot whose fragment is now
    /// in `resolved`. Slots whose fragment is still absent are retained in
    /// the queue (streaming intent: a later register call + flush may still
    /// resolve them; see design §7.4). Per-variant evaluation shares the
    /// same private helper used by the immediate-resolve path, keeping the
    /// fallback semantics in sync across both paths.
    ///
    /// Returns one [`PendingResolution`] per newly-resolved slot, in the
    /// order the slots were queued.
    pub fn flush_pending(&mut self) -> Vec<PendingResolution> {
        let mut resolutions = Vec::new();
        let mut remaining = Vec::with_capacity(self.pending_slots.len());
        for slot in std::mem::take(&mut self.pending_slots) {
            if let Some(info) = self.resolved.get(&slot.fragment_id) {
                resolutions.push(PendingResolution {
                    slot_id: slot.id,
                    value: Some(info.resolve(&slot.request)),
                });
            } else {
                remaining.push(slot);
            }
        }
        self.pending_slots = remaining;
        resolutions
    }

    /// Page-boundary hook — advance the page whose target-* dispatches are
    /// being recorded, resetting the page-local sequence counter to 0.
    ///
    /// Design §7.6 "Slot ID の安定性保証" defines `sequence` as "page 内
    /// 0-indexed" (page-local, not document-wide); a page-boundary call must
    /// restart the local count so a re-run over the same input produces the
    /// same [`TargetSlotId`]s (the byte-identical goal Finding #4, design
    /// §7.4/§7.6, exists for). A no-op if `page_index` already matches the
    /// current value (redundant same-page calls don't corrupt the count of
    /// an in-progress page) — see the "Re-seeded registry" caveat below for
    /// the one case where that guard is *not* what a caller wants.
    ///
    /// # Panics
    ///
    /// Panics if `page_index` is less than the registry's current
    /// `page_index` — i.e. `page_index` must be monotonically
    /// non-decreasing across calls (design §7.6: "page_index は … emit
    /// された順に増加"; same-index calls remain the documented no-op above,
    /// so the enforced contract is non-decreasing, not strictly
    /// increasing). A backward call would otherwise reset `next_sequence`
    /// to 0 while an earlier, still-unresolved `TargetSlot` for that same
    /// `page_index` sits in `pending_slots` (unresolved slots are retained
    /// across flushes, never dropped — see
    /// [`Self::flush_pending`]'s doc), so the very next dispatch would mint
    /// a [`TargetSlotId`] byte-identical to that still-pending one. Two
    /// distinct [`PendingResolution`]s would then carry the same `slot_id`,
    /// breaking the uniqueness/byte-identical guarantee `TargetSlotId`
    /// exists to provide ([`crate::error::TargetSlotId`]'s own doc comment).
    /// `page_index` is driver-supplied internal input, not
    /// external/untrusted data, so a non-monotonic call is a driver
    /// precondition violation rather than adversarial input — this asserts
    /// unconditionally (both debug and release profiles) rather than
    /// `debug_assert!`, since silently corrupting `pending_slots` in a
    /// release build is exactly the failure mode this guard exists to rule
    /// out.
    ///
    /// Same page-boundary role as the sibling
    /// [`super::context::PageContext::begin_page`] hook (different
    /// signature: that one has no driver-supplied index to advance, this one
    /// does), but there is no path from one to the other — `PageContext`
    /// exposes `targets` read-only (no `targets_mut`, see
    /// [`super::context::PageContext::targets`]'s doc), so a driver must
    /// call this directly on an *owned* registry (e.g. the one
    /// `raikiri_dom::target::build_target_registry` returns) before wiring
    /// it into a `PageContext` via
    /// [`super::context::PageContext::set_targets`].
    ///
    /// **Re-seeded registry caveat**: design §7.4's convergence flow seeds a
    /// re-run with a previously-converged registry
    /// (`StreamingConfig::initial_registry` / `BatchConfig::initial_registry`
    /// / `PlanConfig::initial_registry`). If that seed registry's
    /// `page_index` already equals the first page's index (typically 0) but
    /// `next_sequence` is *not* 0 (carried over from the prior run), the
    /// same-value no-op guard above will skip the reset the re-run actually
    /// needs — the re-run's first slot would get the prior run's leftover
    /// sequence instead of restarting at 0, breaking the exact
    /// byte-identical guarantee this method exists to protect. Worse, once a
    /// seed's `page_index` is above the first page's index (the more likely
    /// shape: §7.4's convergence flow seeds with `prev`, the registry as it
    /// stood at the *last* page of the prior run), the monotonic guard above
    /// now turns that same re-run's `begin_page(0)` into a **panic** rather
    /// than a silent reset. No current production code reads
    /// `initial_registry` (placeholder configs only), so neither shape is
    /// reachable today; a future consumer of it is now *required* (not
    /// merely advised) to either reset the seed registry's `page_index` /
    /// `next_sequence` independently before the re-run, or `begin_page` must
    /// gain a force-reset variant — tracked as a follow-up, not fixed here.
    ///
    /// No production driver calls this yet — the per-page walk that would
    /// call it has not landed yet (same gap
    /// `PageContext::begin_page`'s doc already flags). Until a driver calls
    /// it, every slot is tagged to page 0 — correct for a single-page
    /// document, harmless (just uninformative) for a multi-page one.
    pub fn begin_page(&mut self, page_index: u32) {
        let current = self.page_index;
        assert!(
            page_index >= current,
            "TargetRegistry::begin_page: page_index must be monotonically \
             non-decreasing across calls (design §7.6 \"page_index は … \
             emit された順に増加\"); got page_index={page_index} after \
             current page_index={current}. A backward call would reset \
             next_sequence while page {page_index}'s earlier pending slots \
             are still queued, minting a duplicate TargetSlotId."
        );
        if page_index != current {
            self.page_index = page_index;
            self.next_sequence = 0;
        }
    }

    fn next_sequence(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        seq
    }
}

/// Drive a [`TargetRegistry`] from a
/// [`raikiri_style::property::ContentComponent`] — the "working conversion
/// path" for the directive-apply pass.
///
/// Returns `Some(outcome)` for the three target-* variants
/// (`TargetCounter`, `TargetCounters`, `TargetText`), `None` for every
/// other `ContentComponent` variant (`Literal`, `Counter`, `Counters`,
/// `String`, `Attr`, `Content`) and — courtesy of `ContentComponent`'s
/// `#[non_exhaustive]` — any future non-target variants.
///
/// The name conversion `SmolStr → Symbol` happens here so callers don't
/// need to reach into `raikiri-traits::dom` themselves.
pub fn resolve_content_component(
    registry: &mut TargetRegistry,
    cc: &ContentComponent,
) -> Option<ResolveOutcome> {
    match cc {
        ContentComponent::TargetCounter { url, name, style } => {
            Some(registry.resolve_target_counter(url, Symbol::new(name.clone()), style.clone()))
        }
        ContentComponent::TargetCounters {
            url,
            name,
            separator,
            style,
        } => Some(registry.resolve_target_counters(
            url,
            Symbol::new(name.clone()),
            separator,
            style.clone(),
        )),
        ContentComponent::TargetText { url, part } => {
            Some(registry.resolve_target_text(url, *part))
        }
        _ => None,
    }
}

/// Parse a `target-*` URL as a fragment reference.
///
/// Returns `Some(fragment)` for `#fragment` (leading `#` stripped, empty
/// fragment rejected), `None` otherwise. Cross-document / external URL
/// resolution is out of scope for raikiri (per-document paginate).
fn parse_fragment(url: &str) -> Option<&str> {
    let rest = url.strip_prefix('#')?;
    (!rest.is_empty()).then_some(rest)
}

/// Format one counter value.
///
/// Implements the CSS Counter Styles Level 3 "generate a counter
/// representation" algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#generate-a-counter> for
/// [`CounterStyle::Decimal`] and, via [`format_named_counter`], every named
/// style in CSS Counter Styles L3 §6 "Simple Predefined Counter Styles"
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters> and §7
/// "Complex Predefined Counter Styles"
/// <https://www.w3.org/TR/css-counter-styles-3/#complex-predefined-counters>
/// except `disclosure-open` / `disclosure-closed` (see reason 3 below).
/// Landed in two passes: `decimal-leading-zero`, `lower-roman` /
/// `upper-roman`, `lower-alpha` / `upper-alpha` (+ the `lower-latin` /
/// `upper-latin` aliases §6 defines with identical symbol tables), `disc` /
/// `circle` / `square` in an earlier pass; the remainder — §6.1 Numeric,
/// §6.2 Alphabetic, §6.4 Fixed, §7.1 Longhand East Asian, §7.2 Ethiopic
/// Numeric — in a later pass. See
/// [`format_named_counter`]'s doc for the full per-family breakdown and
/// citations. ("Implemented" here means implementation coverage, not a
/// registry boundary: every §6/§7 predefined style is a UA-stylesheet rule,
/// not an author `@counter-style` registration, so none of them need a
/// registry — implemented or not. Cf. reason 2 below, which scopes the
/// registry dependency to author-defined custom idents.)
///
/// `CounterStyle::Named` beyond that set falls back to `decimal` for three
/// distinct reasons, and only the first is actually generate-a-counter step
/// 1 ("If the counter style is unknown, ... generate a counter
/// representation using the decimal style"):
///
/// 1. **Genuinely unknown name** — not a CSS Counter Styles L3 keyword at
///    all. Step 1 applies verbatim; decimal is spec-correct here.
/// 2. **A custom `@counter-style` rule** — a valid `<custom-ident>` per
///    spec, but *unreachable* today: raikiri-style has no `@counter-style`
///    parser/registry to resolve it against (tracked as a follow-up).
/// 3. **`disclosure-open` / `disclosure-closed`** — the only §6/§7
///    predefined styles still unimplemented. Deliberately
///    deferred, not a fallback bug: §6.3
///    <https://www.w3.org/TR/css-counter-styles-3/#simple-symbolic>'s
///    normative stylesheet fragment leaves their `symbols` descriptor
///    unset ("for symbols, see normative text below"), and the referenced
///    text only says the marker "must be an image or character suitable
///    for indicating" open/closed state, then gives U+25B8/U+25C2/U+25BE
///    as something a UA "might use" — not a mandated codepoint — and
///    explicitly ties the choice to bidi writing-mode direction. There is
///    no single normative codepoint to pin, and `format_named_counter`'s
///    signature (`value: i32, name: &str`) has no writing-mode/direction
///    input to pick a directional variant with. Landing a guessed glyph
///    here would be exactly the "complete but wrong" outcome this work was
///    meant to avoid; tracked as a residual scope item for follow-up.
///
/// **Visibility**: `pub(crate)` (widened from private) — see
/// [`join_counter_stack`]'s doc for why (sibling `page::context` module
/// needs this for real `string-set` text resolution).
pub(crate) fn format_counter(value: i32, style: &CounterStyle) -> String {
    match style {
        CounterStyle::Decimal => format_decimal(value),
        CounterStyle::Named(name) => format_named_counter(value, name),
        // `CounterStyle` is `#[non_exhaustive]` (raikiri-style may add
        // variants later) — an unrecognized future variant is exactly the
        // generate-a-counter step 1 "unknown style" case, so it gets the
        // same decimal fallback as an unrecognized `Named` string.
        // cov:ignore: unreachable today — `CounterStyle` currently only
        // constructs `Decimal` / `Named` (raikiri-style
        // property.rs:counter_style_from_ident), so this arm has no live
        // input until a third variant lands upstream; required only to
        // satisfy the non_exhaustive match-exhaustiveness check.
        _ => format_decimal(value),
    }
}

/// Bare decimal representation — CSS Counter Styles L3 `numeric` system with
/// the `decimal` `symbols` table (digits `0`-`9`) and default `negative:
/// "-"` prefix <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>.
/// Rust's integer `Display` already produces exactly this (base-10 digits,
/// `-` prefix for negative, no separate sign for zero).
fn format_decimal(value: i32) -> String {
    format!("{value}")
}

/// Dispatch the predefined [`CounterStyle::Named`] styles implemented here
/// (see [`format_counter`] doc for the overall implemented/deferred split
/// and the three distinct reasons an unmatched name falls back to
/// `decimal`).
///
/// Matching is ASCII case-insensitive, consistent with how
/// `raikiri_style::property`'s parser already treats the `decimal` keyword
/// case-insensitively — CSS keyword idents are case-insensitive generally,
/// this file just extends the same rule to the rest of §6/§7.
///
/// **Dispatch shape diverges from the og2 baseline (37n: sibling-convention
/// divergence noted).** The 8 og2-era names above are matched with a plain
/// `if`/`else if` chain (one `eq_ignore_ascii_case` per name, sometimes two
/// for an alias pair) — readable at that scale. ce3k adds ~40 more names
/// across 6 more algorithm families; repeating that shape per name would
/// mean 40+ near-duplicate arms and would scatter each family's single
/// citation across many call sites. Instead, ce3k-added families are each a
/// `const &[(&str, ...)]` table (name → per-style data) plus one shared
/// formatter function, looked up with `.iter().find(|(n, _)| name
/// .eq_ignore_ascii_case(n))` — same no-allocation case-insensitive match
/// per name, just table-driven instead of branch-driven. Each table's own
/// doc comment carries the `#anchor` citation for every name in it, so the
/// per-name spec citation requirement is still met once per family rather
/// than once per arm.
fn format_named_counter(value: i32, name: &str) -> String {
    if name.eq_ignore_ascii_case("decimal-leading-zero") {
        format_decimal_leading_zero(value)
    } else if name.eq_ignore_ascii_case("lower-roman") {
        format_roman(value, false).unwrap_or_else(|| format_decimal(value))
    } else if name.eq_ignore_ascii_case("upper-roman") {
        format_roman(value, true).unwrap_or_else(|| format_decimal(value))
    } else if name.eq_ignore_ascii_case("lower-alpha") || name.eq_ignore_ascii_case("lower-latin") {
        format_alphabetic(value, b'a').unwrap_or_else(|| format_decimal(value))
    } else if name.eq_ignore_ascii_case("upper-alpha") || name.eq_ignore_ascii_case("upper-latin") {
        format_alphabetic(value, b'A').unwrap_or_else(|| format_decimal(value))
    } else if name.eq_ignore_ascii_case("disc") {
        // `system: cyclic; symbols: \2022` — a single-symbol cyclic system
        // always renders the same glyph regardless of value (range
        // -infinity..infinity, no fallback path). `suffix: " "` in the
        // §6.3 (#simple-symbolic) block is deliberately not reproduced
        // here: generate-a-counter's prefix/suffix are for the ::marker
        // box, not for the string `counter()`/`counters()` (and by
        // extension `target-counter()`/`target-counters()`) return.
        "\u{2022}".to_string()
    } else if name.eq_ignore_ascii_case("circle") {
        "\u{25E6}".to_string()
    } else if name.eq_ignore_ascii_case("square") {
        "\u{25AA}".to_string()
    } else if let Some((_, digits)) = NUMERIC_DIGIT_STYLES
        .iter()
        .find(|(n, _)| name.eq_ignore_ascii_case(n))
    {
        format_numeric_digits(value, digits)
    } else if name.eq_ignore_ascii_case("cjk-decimal") {
        format_cjk_decimal(value).unwrap_or_else(|| format_decimal(value))
    } else if let Some((_, table, range)) = ADDITIVE_NUMERIC_STYLES
        .iter()
        .find(|(n, _, _)| name.eq_ignore_ascii_case(n))
    {
        format_additive_named(value, table, *range).unwrap_or_else(|| format_decimal(value))
    } else if let Some((_, symbols)) = ALPHABETIC_EXTENDED_STYLES
        .iter()
        .find(|(n, _)| name.eq_ignore_ascii_case(n))
    {
        format_alphabetic_symbols(value, symbols).unwrap_or_else(|| format_decimal(value))
    } else if let Some((_, symbols)) = FIXED_STYLES
        .iter()
        .find(|(n, _)| name.eq_ignore_ascii_case(n))
    {
        format_fixed(value, symbols).unwrap_or_else(|| format_decimal(value))
    } else if let Some((_, table, negative)) = LONGHAND_ADDITIVE_STYLES
        .iter()
        .find(|(n, _, _)| name.eq_ignore_ascii_case(n))
    {
        // Fallback chain diverges from every other arm's flat
        // `.unwrap_or_else(|| format_decimal(value))` (37n: sibling
        // divergence noted). §7.1's opening paragraph
        // <https://www.w3.org/TR/css-counter-styles-3/#complex-predefined-counters>
        // states plainly, for the section as a whole: "all of the counter
        // styles defined in this section are defined to have a range of
        // -9999 to 9999 ... Outside the ... range, the fallback is
        // cjk-decimal." The Japanese `@counter-style` blocks restate this
        // with an explicit `fallback: cjk-decimal;` descriptor; the Korean
        // blocks in the same normative stylesheet fragment omit that
        // descriptor line, which taken alone would default it to the
        // `fallback` descriptor's own initial value `decimal`
        // (`#descdef-counter-style-fallback`) — but the section-level
        // sentence is written as applying to every style "defined in this
        // section", Japanese and Korean alike, so the omission reads as
        // stylesheet-fragment economy, not a per-style override. Routing
        // out-of-range values here to cjk-decimal first, rather than
        // straight to decimal, follows that section-level statement;
        // cjk-decimal's own range (`0 infinite`) then rejects negative
        // values in turn, falling through to plain decimal for those — a
        // real two-hop chain, not just a single substitution.
        format_longhand_additive(value, table, negative)
            .or_else(|| format_cjk_decimal(value))
            .unwrap_or_else(|| format_decimal(value))
    } else if let Some((_, table)) = CHINESE_DIGIT_MARKER_STYLES
        .iter()
        .find(|(n, _)| name.eq_ignore_ascii_case(n))
    {
        // Same two-hop fallback chain as the longhand-additive arm above,
        // and for the same §7.1 opening-paragraph reason (§7.1.3
        // restates it again, specific to the four Chinese styles: "the
        // fallback is cjk-decimal").
        format_chinese_digit_marker(value, table)
            .or_else(|| format_cjk_decimal(value))
            .unwrap_or_else(|| format_decimal(value))
    } else if name.eq_ignore_ascii_case("ethiopic-numeric") {
        format_ethiopic_numeric(value).unwrap_or_else(|| format_decimal(value))
    } else {
        // Unmatched named style — decimal fallback for one of the three
        // reasons enumerated in the `format_counter` doc (genuinely
        // unknown / unreachable custom @counter-style /
        // disclosure-open/-closed's undefined-codepoint deferral).
        format_decimal(value)
    }
}

/// `decimal-leading-zero` — `system: extends decimal; pad: 2 "0"`
/// <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>,
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-pad>.
/// generate-a-counter builds the initial representation from the *absolute*
/// value (step 3), pads it to width 2 with `'0'` (step 4), then applies the
/// negative sign outside the padding (step 5) — so `-1` is `"-01"`, not
/// `"0-1"`.
fn format_decimal_leading_zero(value: i32) -> String {
    let magnitude = format!("{:02}", value.unsigned_abs());
    if value < 0 {
        format!("-{magnitude}")
    } else {
        magnitude
    }
}

/// `additive` system weight/symbol table shared by `lower-roman` /
/// `upper-roman`
/// <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>. Symbols are
/// authored lower-case; `upper-roman` upper-cases the composed result
/// (pure-ASCII letters, so this is equivalent to the spec's separate
/// upper-case `additive-symbols` table).
const ROMAN_ADDITIVE: [(i32, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

/// `additive` system algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#additive-system>, specialized
/// to [`ROMAN_ADDITIVE`] and the `range: 1 3999` descriptor shared by
/// `lower-roman` / `upper-roman`
/// <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>. Returns
/// `None` when `value` is outside that range, signalling the caller to fall
/// back to `decimal` per `#counter-style-range` /
/// `#generate-a-counter` step 2 (range check uses the *original* signed
/// value, before any absolute-value substitution).
fn format_roman(value: i32, upper: bool) -> Option<String> {
    if !(1..=3999).contains(&value) {
        return None;
    }
    let out = format_additive_system(value, &ROMAN_ADDITIVE)?;
    Some(if upper { out.to_ascii_uppercase() } else { out })
}

/// `alphabetic` system algorithm (bijective base-26)
/// <https://www.w3.org/TR/css-counter-styles-3/#alphabetic-system>,
/// specialized to the 26-letter Latin `symbols` table shared by
/// `lower-alpha` / `upper-alpha` (and the `lower-latin` / `upper-latin`
/// aliases) <https://www.w3.org/TR/css-counter-styles-3/#simple-alphabetic>.
/// `first_symbol` is `b'a'` or `b'A'`. The system's implicit range is
/// strictly-positive integers (`#counter-style-range` default for
/// `alphabetic`); returns `None` outside that range so the caller falls
/// back to `decimal`.
fn format_alphabetic(value: i32, first_symbol: u8) -> Option<String> {
    if value < 1 {
        return None;
    }
    let mut remaining = value;
    let mut digits = Vec::new();
    while remaining != 0 {
        remaining -= 1;
        let digit = (remaining % 26) as u8;
        digits.push((first_symbol + digit) as char);
        remaining /= 26;
    }
    digits.reverse();
    Some(digits.into_iter().collect())
}

// ── §6.1 Numeric: `numeric`-system digit tables ─────────────────────────

/// `numeric` system algorithm <https://www.w3.org/TR/css-counter-styles-3/#numeric-system>,
/// specialized to the 10-digit place-value tables §6.1
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters> defines
/// for: `arabic-indic`, `bengali`, `cambodian` (`khmer` is `system: extends
/// cambodian` — identical table, listed as its own entry below rather than
/// matched as an alias, since unlike `lower-latin`/`upper-latin` this file
/// doesn't special-case aliases inline), `devanagari`, `gujarati`,
/// `gurmukhi`, `kannada`, `lao`, `malayalam`, `mongolian`, `myanmar`,
/// `oriya`, `persian`, `tamil`, `telugu`, `thai`, `tibetan`. `cjk-decimal`
/// is also `system: numeric` with this same digit-substitution algorithm
/// but is dispatched separately ([`format_cjk_decimal`]) because its
/// `range: 0 infinite` descriptor excludes negative values, unlike every
/// other numeric-system style here (which use the system's default,
/// unrestricted range — see [`format_numeric_digits`]).
///
/// Codepoints transcribed from the normative `@counter-style` stylesheet
/// fragment at the end of §6.1; cross-checked against each style's own
/// `(e.g., …)` reference values at 1, 2, 3, 98, 99, 100 given earlier in
/// the same section.
/// `cambodian`'s digit table, shared as a single const with its `system:
/// extends cambodian` alias `khmer` (both dispatch-table entries below copy
/// this `Copy` array by value, but from one source of truth, not two
/// independently hand-copied literals) (37n: unlike `lower-latin`/
/// `upper-latin`, which
/// alias-match by name in [`format_named_counter`]'s dispatch, `khmer` gets
/// its own `NUMERIC_DIGIT_STYLES` entry — see the rationale on that table
/// below) so the two names can never drift out of sync with each other.
const CAMBODIAN_KHMER_DIGITS: [char; 10] = [
    '\u{17e0}', '\u{17e1}', '\u{17e2}', '\u{17e3}', '\u{17e4}', '\u{17e5}', '\u{17e6}', '\u{17e7}',
    '\u{17e8}', '\u{17e9}',
];

const NUMERIC_DIGIT_STYLES: &[(&str, [char; 10])] = &[
    (
        "arabic-indic",
        [
            '\u{660}', '\u{661}', '\u{662}', '\u{663}', '\u{664}', '\u{665}', '\u{666}', '\u{667}',
            '\u{668}', '\u{669}',
        ],
    ), // ٠ ١ ٢ ٣ ٤ ٥ ٦ ٧ ٨ ٩
    (
        "bengali",
        [
            '\u{9e6}', '\u{9e7}', '\u{9e8}', '\u{9e9}', '\u{9ea}', '\u{9eb}', '\u{9ec}', '\u{9ed}',
            '\u{9ee}', '\u{9ef}',
        ],
    ), // ০ ১ ২ ৩ ৪ ৫ ৬ ৭ ৮ ৯
    ("cambodian", CAMBODIAN_KHMER_DIGITS), // ០ ១ ២ ៣ ៤ ៥ ៦ ៧ ៨ ៩
    ("khmer", CAMBODIAN_KHMER_DIGITS), // `system: extends cambodian` — same const, not a hand-copied duplicate
    (
        "devanagari",
        [
            '\u{966}', '\u{967}', '\u{968}', '\u{969}', '\u{96a}', '\u{96b}', '\u{96c}', '\u{96d}',
            '\u{96e}', '\u{96f}',
        ],
    ), // ० १ २ ३ ४ ५ ६ ७ ८ ९
    (
        "gujarati",
        [
            '\u{ae6}', '\u{ae7}', '\u{ae8}', '\u{ae9}', '\u{aea}', '\u{aeb}', '\u{aec}', '\u{aed}',
            '\u{aee}', '\u{aef}',
        ],
    ), // ૦ ૧ ૨ ૩ ૪ ૫ ૬ ૭ ૮ ૯
    (
        "gurmukhi",
        [
            '\u{a66}', '\u{a67}', '\u{a68}', '\u{a69}', '\u{a6a}', '\u{a6b}', '\u{a6c}', '\u{a6d}',
            '\u{a6e}', '\u{a6f}',
        ],
    ), // ੦ ੧ ੨ ੩ ੪ ੫ ੬ ੭ ੮ ੯
    (
        "kannada",
        [
            '\u{ce6}', '\u{ce7}', '\u{ce8}', '\u{ce9}', '\u{cea}', '\u{ceb}', '\u{cec}', '\u{ced}',
            '\u{cee}', '\u{cef}',
        ],
    ), // ೦ ೧ ೨ ೩ ೪ ೫ ೬ ೭ ೮ ೯
    (
        "lao",
        [
            '\u{ed0}', '\u{ed1}', '\u{ed2}', '\u{ed3}', '\u{ed4}', '\u{ed5}', '\u{ed6}', '\u{ed7}',
            '\u{ed8}', '\u{ed9}',
        ],
    ), // ໐ ໑ ໒ ໓ ໔ ໕ ໖ ໗ ໘ ໙
    (
        "malayalam",
        [
            '\u{d66}', '\u{d67}', '\u{d68}', '\u{d69}', '\u{d6a}', '\u{d6b}', '\u{d6c}', '\u{d6d}',
            '\u{d6e}', '\u{d6f}',
        ],
    ), // ൦ ൧ ൨ ൩ ൪ ൫ ൬ ൭ ൮ ൯
    (
        "mongolian",
        [
            '\u{1810}', '\u{1811}', '\u{1812}', '\u{1813}', '\u{1814}', '\u{1815}', '\u{1816}',
            '\u{1817}', '\u{1818}', '\u{1819}',
        ],
    ), // ᠐ ᠑ ᠒ ᠓ ᠔ ᠕ ᠖ ᠗ ᠘ ᠙
    (
        "myanmar",
        [
            '\u{1040}', '\u{1041}', '\u{1042}', '\u{1043}', '\u{1044}', '\u{1045}', '\u{1046}',
            '\u{1047}', '\u{1048}', '\u{1049}',
        ],
    ), // ၀ ၁ ၂ ၃ ၄ ၅ ၆ ၇ ၈ ၉
    (
        "oriya",
        [
            '\u{b66}', '\u{b67}', '\u{b68}', '\u{b69}', '\u{b6a}', '\u{b6b}', '\u{b6c}', '\u{b6d}',
            '\u{b6e}', '\u{b6f}',
        ],
    ), // ୦ ୧ ୨ ୩ ୪ ୫ ୬ ୭ ୮ ୯
    (
        "persian",
        [
            '\u{6f0}', '\u{6f1}', '\u{6f2}', '\u{6f3}', '\u{6f4}', '\u{6f5}', '\u{6f6}', '\u{6f7}',
            '\u{6f8}', '\u{6f9}',
        ],
    ), // ۰ ۱ ۲ ۳ ۴ ۵ ۶ ۷ ۸ ۹
    (
        "tamil",
        [
            '\u{be6}', '\u{be7}', '\u{be8}', '\u{be9}', '\u{bea}', '\u{beb}', '\u{bec}', '\u{bed}',
            '\u{bee}', '\u{bef}',
        ],
    ), // ௦ ௧ ௨ ௩ ௪ ௫ ௬ ௭ ௮ ௯
    (
        "telugu",
        [
            '\u{c66}', '\u{c67}', '\u{c68}', '\u{c69}', '\u{c6a}', '\u{c6b}', '\u{c6c}', '\u{c6d}',
            '\u{c6e}', '\u{c6f}',
        ],
    ), // ౦ ౧ ౨ ౩ ౪ ౫ ౬ ౭ ౮ ౯
    (
        "thai",
        [
            '\u{e50}', '\u{e51}', '\u{e52}', '\u{e53}', '\u{e54}', '\u{e55}', '\u{e56}', '\u{e57}',
            '\u{e58}', '\u{e59}',
        ],
    ), // ๐ ๑ ๒ ๓ ๔ ๕ ๖ ๗ ๘ ๙
    (
        "tibetan",
        [
            '\u{f20}', '\u{f21}', '\u{f22}', '\u{f23}', '\u{f24}', '\u{f25}', '\u{f26}', '\u{f27}',
            '\u{f28}', '\u{f29}',
        ],
    ), // ༠ ༡ ༢ ༣ ༤ ༥ ༦ ༧ ༨ ༩
];

/// Render `value` in a positional base-10 numeral system whose digit glyphs
/// are `digits` (`digits[0]` = the glyph for `0`, …, `digits[9]` = the
/// glyph for `9`) — the `numeric` system algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#numeric-system> is, for a
/// base equal to the ASCII decimal base, exactly digit-by-digit
/// substitution over the unsigned magnitude's own decimal `Display`
/// output. Unlike [`format_roman`] / [`format_alphabetic`], `numeric` "is
/// defined over all counter values" (§3.1.5) — no range check, no
/// `Option`. All of these styles use the default `negative: "-"` (ASCII
/// hyphen-minus) prefix; none of the §6.1 `@counter-style` blocks override
/// it.
fn format_numeric_digits(value: i32, digits: &[char; 10]) -> String {
    let magnitude = value.unsigned_abs();
    let mut out = String::new();
    if value < 0 {
        out.push('-');
    }
    for ch in magnitude.to_string().chars() {
        let digit = ch
            .to_digit(10)
            .expect("decimal Display output is 0-9 ASCII digits only");
        out.push(digits[digit as usize]);
    }
    out
}

/// `cjk-decimal`'s digit table
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters> — same
/// `numeric` system as [`NUMERIC_DIGIT_STYLES`], dispatched separately
/// because of the range difference documented on [`format_numeric_digits`].
const CJK_DECIMAL_DIGITS: [char; 10] = [
    '\u{3007}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}', '\u{4e03}',
    '\u{516b}', '\u{4e5d}',
]; // 〇 一 二 三 四 五 六 七 八 九

/// `cjk-decimal`'s `range: 0 infinite` override
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters>
/// (every other `numeric`-system style here keeps the system's default,
/// unrestricted range) — negative values are out of range and must use
/// this style's own `fallback`, which the `@counter-style cjk-decimal`
/// block doesn't set, so it takes the `fallback` descriptor's initial
/// value `decimal`
/// <https://www.w3.org/TR/css-counter-styles-3/#descdef-counter-style-fallback>.
/// Returns `None` for negative `value` so [`format_named_counter`]'s
/// direct-dispatch caller falls back to `decimal`, and so the §7.1
/// longhand-East-Asian callers (whose own fallback chain routes through
/// this function first) fall through to `decimal` in turn when the value
/// is still negative after that first hop.
fn format_cjk_decimal(value: i32) -> Option<String> {
    if value < 0 {
        return None;
    }
    Some(format_numeric_digits(value, &CJK_DECIMAL_DIGITS))
}

// ── §6.1 Numeric: `additive`-system styles (armenian, georgian, hebrew) ─

/// `(weight, symbol)` table shape shared by [`format_additive_system`] and
/// every table that feeds it ([`ROMAN_ADDITIVE`]-shaped, but named here so
/// the `(&str, AdditiveSymbols, ...)` dispatch-table types below don't trip
/// clippy's `type_complexity` lint on the raw nested-slice-of-tuples form.
type AdditiveSymbols = &'static [(i32, &'static str)];

/// Generic `additive` system algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#additive-system>, run on
/// `magnitude` (already non-negative — callers are responsible for the
/// generate-a-counter step 3 "use the absolute value" substitution and the
/// step 5 negative-sign wrap).
///
/// This is the general form, including the algorithm's `value == 0` case
/// ("If symbol list contains a tuple with a weight of zero, append that
/// tuple's counter symbol") because the §7.1 longhand East Asian tables
/// (see the `LONGHAND_ADDITIVE_STYLES` dispatch arm in
/// [`format_named_counter`]) include an explicit `0` tuple and their range
/// includes 0. [`format_roman`] wraps this function (its `lower-roman`/
/// `upper-roman` range starts at 1 and never reaches `magnitude == 0`, so
/// the zero-weight branch is simply never taken there) and applies its own
/// pure-ASCII upper-casing as a post-processing step on the returned
/// `String`, rather than duplicating this loop.
///
/// Returns `None` when no combination of tuples sums exactly to
/// `magnitude` (the algorithm's own "assertion: value is still non-zero"
/// fallback path) — unreachable for every table used in this file, since
/// all of them cover their declared range exhaustively, but kept as a real
/// fallback rather than a panic because the algorithm itself defines it as
/// a fallback-style case, not a logic error.
fn format_additive_system(magnitude: i32, table: AdditiveSymbols) -> Option<String> {
    if magnitude == 0 {
        return table
            .iter()
            .find(|(weight, _)| *weight == 0)
            .map(|(_, symbol)| (*symbol).to_string());
    }
    let mut remaining = magnitude;
    let mut out = String::new();
    for &(weight, symbol) in table {
        if weight == 0 || weight > remaining {
            continue;
        }
        let reps = remaining / weight;
        for _ in 0..reps {
            out.push_str(symbol);
        }
        remaining -= weight * reps;
        if remaining == 0 {
            return Some(out);
        }
    }
    None
}

/// §6.1 `additive`-system predefined styles beyond `lower-roman`/
/// `upper-roman`
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters>:
/// `armenian` (traditional/uppercase), `upper-armenian` (`system: extends
/// armenian` — identical table, listed as its own entry per the same
/// no-separate-alias-matching rationale as `khmer` on
/// [`NUMERIC_DIGIT_STYLES`]), `lower-armenian`, `georgian`, `hebrew`. Each
/// entry is `(name, additive-symbols table, range)`. None of these ranges
/// include 0 or negative values, so [`format_additive_system`]'s
/// `magnitude == 0` branch never fires for this table and the "use
/// absolute value" / negative-sign-wrap steps of generate-a-counter never
/// apply — [`format_additive_named`] passes `value` straight through as
/// the (always non-negative, once in range) magnitude.
///
/// Codepoints transcribed from the normative `@counter-style` stylesheet
/// fragment in §6.1; `armenian`/`georgian`/`hebrew` cross-checked against
/// their own `(e.g., …)` reference values at 1, 2, 3, 98, 99, 100 —
/// `hebrew`'s 98 = weight-90 + weight-8 = צח, matching the dfn line's
/// `..., צח, צט, ק`; `georgian`'s 98 = weight-90 (ჟ) + weight-8 (ჱ, a
/// dedicated numeral letter, not the 9th-place თ) = ჟჱ, matching `...,
/// ჟჱ, ჟთ, რ`.
const ADDITIVE_NUMERIC_STYLES: &[(&str, AdditiveSymbols, (i32, i32))] = &[
    ("armenian", &ARMENIAN_ADDITIVE, (1, 9999)),
    ("upper-armenian", &ARMENIAN_ADDITIVE, (1, 9999)),
    ("lower-armenian", &LOWER_ARMENIAN_ADDITIVE, (1, 9999)),
    ("georgian", &GEORGIAN_ADDITIVE, (1, 19999)),
    ("hebrew", &HEBREW_ADDITIVE, (1, 10999)),
];

const ARMENIAN_ADDITIVE: [(i32, &str); 36] = [
    (9000, "\u{554}"),
    (8000, "\u{553}"),
    (7000, "\u{552}"),
    (6000, "\u{551}"),
    (5000, "\u{550}"),
    (4000, "\u{54f}"),
    (3000, "\u{54e}"),
    (2000, "\u{54d}"),
    (1000, "\u{54c}"),
    (900, "\u{54b}"),
    (800, "\u{54a}"),
    (700, "\u{549}"),
    (600, "\u{548}"),
    (500, "\u{547}"),
    (400, "\u{546}"),
    (300, "\u{545}"),
    (200, "\u{544}"),
    (100, "\u{543}"),
    (90, "\u{542}"),
    (80, "\u{541}"),
    (70, "\u{540}"),
    (60, "\u{53f}"),
    (50, "\u{53e}"),
    (40, "\u{53d}"),
    (30, "\u{53c}"),
    (20, "\u{53b}"),
    (10, "\u{53a}"),
    (9, "\u{539}"),
    (8, "\u{538}"),
    (7, "\u{537}"),
    (6, "\u{536}"),
    (5, "\u{535}"),
    (4, "\u{534}"),
    (3, "\u{533}"),
    (2, "\u{532}"),
    (1, "\u{531}"),
]; // 9000:Ք 8000:Փ 7000:Ւ 6000:Ց 5000:Ր 4000:Տ 3000:Վ 2000:Ս 1000:Ռ 900:Ջ 800:Պ 700:Չ 600:Ո 500:Շ 400:Ն 300:Յ 200:Մ 100:Ճ 90:Ղ 80:Ձ 70:Հ 60:Կ 50:Ծ 40:Խ 30:Լ 20:Ի 10:Ժ 9:Թ 8:Ը 7:Է 6:Զ 5:Ե 4:Դ 3:Գ 2:Բ 1:Ա

const LOWER_ARMENIAN_ADDITIVE: [(i32, &str); 36] = [
    (9000, "\u{584}"),
    (8000, "\u{583}"),
    (7000, "\u{582}"),
    (6000, "\u{581}"),
    (5000, "\u{580}"),
    (4000, "\u{57f}"),
    (3000, "\u{57e}"),
    (2000, "\u{57d}"),
    (1000, "\u{57c}"),
    (900, "\u{57b}"),
    (800, "\u{57a}"),
    (700, "\u{579}"),
    (600, "\u{578}"),
    (500, "\u{577}"),
    (400, "\u{576}"),
    (300, "\u{575}"),
    (200, "\u{574}"),
    (100, "\u{573}"),
    (90, "\u{572}"),
    (80, "\u{571}"),
    (70, "\u{570}"),
    (60, "\u{56f}"),
    (50, "\u{56e}"),
    (40, "\u{56d}"),
    (30, "\u{56c}"),
    (20, "\u{56b}"),
    (10, "\u{56a}"),
    (9, "\u{569}"),
    (8, "\u{568}"),
    (7, "\u{567}"),
    (6, "\u{566}"),
    (5, "\u{565}"),
    (4, "\u{564}"),
    (3, "\u{563}"),
    (2, "\u{562}"),
    (1, "\u{561}"),
]; // 9000:ք 8000:փ 7000:ւ 6000:ց 5000:ր 4000:տ 3000:վ 2000:ս 1000:ռ 900:ջ 800:պ 700:չ 600:ո 500:շ 400:ն 300:յ 200:մ 100:ճ 90:ղ 80:ձ 70:հ 60:կ 50:ծ 40:խ 30:լ 20:ի 10:ժ 9:թ 8:ը 7:է 6:զ 5:ե 4:դ 3:գ 2:բ 1:ա

const GEORGIAN_ADDITIVE: [(i32, &str); 37] = [
    (10000, "\u{10f5}"),
    (9000, "\u{10f0}"),
    (8000, "\u{10ef}"),
    (7000, "\u{10f4}"),
    (6000, "\u{10ee}"),
    (5000, "\u{10ed}"),
    (4000, "\u{10ec}"),
    (3000, "\u{10eb}"),
    (2000, "\u{10ea}"),
    (1000, "\u{10e9}"),
    (900, "\u{10e8}"),
    (800, "\u{10e7}"),
    (700, "\u{10e6}"),
    (600, "\u{10e5}"),
    (500, "\u{10e4}"),
    (400, "\u{10f3}"),
    (300, "\u{10e2}"),
    (200, "\u{10e1}"),
    (100, "\u{10e0}"),
    (90, "\u{10df}"),
    (80, "\u{10de}"),
    (70, "\u{10dd}"),
    (60, "\u{10f2}"),
    (50, "\u{10dc}"),
    (40, "\u{10db}"),
    (30, "\u{10da}"),
    (20, "\u{10d9}"),
    (10, "\u{10d8}"),
    (9, "\u{10d7}"),
    (8, "\u{10f1}"),
    (7, "\u{10d6}"),
    (6, "\u{10d5}"),
    (5, "\u{10d4}"),
    (4, "\u{10d3}"),
    (3, "\u{10d2}"),
    (2, "\u{10d1}"),
    (1, "\u{10d0}"),
]; // 10000:ჵ 9000:ჰ 8000:ჯ 7000:ჴ 6000:ხ 5000:ჭ 4000:წ 3000:ძ 2000:ც 1000:ჩ 900:შ 800:ყ 700:ღ 600:ქ 500:ფ 400:ჳ 300:ტ 200:ს 100:რ 90:ჟ 80:პ 70:ო 60:ჲ 50:ნ 40:მ 30:ლ 20:კ 10:ი 9:თ 8:ჱ 7:ზ 6:ვ 5:ე 4:დ 3:გ 2:ბ 1:ა

const HEBREW_ADDITIVE: [(i32, &str); 37] = [
    (10000, "\u{5d9}\u{5f3}"),
    (9000, "\u{5d8}\u{5f3}"),
    (8000, "\u{5d7}\u{5f3}"),
    (7000, "\u{5d6}\u{5f3}"),
    (6000, "\u{5d5}\u{5f3}"),
    (5000, "\u{5d4}\u{5f3}"),
    (4000, "\u{5d3}\u{5f3}"),
    (3000, "\u{5d2}\u{5f3}"),
    (2000, "\u{5d1}\u{5f3}"),
    (1000, "\u{5d0}\u{5f3}"),
    (400, "\u{5ea}"),
    (300, "\u{5e9}"),
    (200, "\u{5e8}"),
    (100, "\u{5e7}"),
    (90, "\u{5e6}"),
    (80, "\u{5e4}"),
    (70, "\u{5e2}"),
    (60, "\u{5e1}"),
    (50, "\u{5e0}"),
    (40, "\u{5de}"),
    (30, "\u{5dc}"),
    (20, "\u{5db}"),
    (19, "\u{5d9}\u{5d8}"),
    (18, "\u{5d9}\u{5d7}"),
    (17, "\u{5d9}\u{5d6}"),
    (16, "\u{5d8}\u{5d6}"),
    (15, "\u{5d8}\u{5d5}"),
    (10, "\u{5d9}"),
    (9, "\u{5d8}"),
    (8, "\u{5d7}"),
    (7, "\u{5d6}"),
    (6, "\u{5d5}"),
    (5, "\u{5d4}"),
    (4, "\u{5d3}"),
    (3, "\u{5d2}"),
    (2, "\u{5d1}"),
    (1, "\u{5d0}"),
]; // 10000:י׳ 9000:ט׳ 8000:ח׳ 7000:ז׳ 6000:ו׳ 5000:ה׳ 4000:ד׳ 3000:ג׳ 2000:ב׳ 1000:א׳ 400:ת 300:ש 200:ר 100:ק 90:צ 80:פ 70:ע 60:ס 50:נ 40:מ 30:ל 20:כ 19:יט(manual override) 18:יח 17:יז 16:טז(manual) 15:טו(manual) 10:י 9:ט 8:ח 7:ז 6:ו 5:ה 4:ד 3:ג 2:ב 1:א

/// Range-check + dispatch wrapper for [`ADDITIVE_NUMERIC_STYLES`] entries —
/// out-of-range `value` (including 0 and negative, for every table here)
/// returns `None` so the caller falls back to `decimal`, matching
/// [`format_roman`]'s `range: 1 3999` handling.
fn format_additive_named(value: i32, table: AdditiveSymbols, range: (i32, i32)) -> Option<String> {
    if !(range.0..=range.1).contains(&value) {
        return None;
    }
    format_additive_system(value, table)
}

// ── §6.2 Alphabetic: `alphabetic`-system styles beyond lower/upper-alpha ─

/// §6.2 `alphabetic`-system predefined styles beyond `lower-alpha`/
/// `upper-alpha` (+ `lower-latin`/`upper-latin` aliases)
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters>:
/// `lower-greek`, `hiragana`, `hiragana-iroha`, `katakana`,
/// `katakana-iroha`. Symbol-table lengths vary per style (`lower-greek`
/// skips final sigma ς, 24 symbols; `hiragana`/`katakana` 48 symbols each;
/// the `-iroha`-ordered variants drop ん/ン, 47 symbols each), unlike
/// `format_alphabetic`'s fixed 26-letter Latin range — see
/// [`format_alphabetic_symbols`] for the generalized algorithm.
///
/// Codepoints transcribed from the normative `@counter-style` stylesheet
/// fragment in §6.2; symbol counts (and therefore the bijective-base-N
/// wrap point) cross-checked against each style's own `(e.g., …)`
/// reference values — `lower-greek`'s `..., ω, αα, αβ` confirms 24 symbols
/// (ω is the 24th), `hiragana`'s `..., ん, ああ, あい` confirms 48,
/// `hiragana-iroha`'s `..., す, いい, いろ` confirms 47 (す, not ん, is the
/// 47th symbol in iroha order).
const ALPHABETIC_EXTENDED_STYLES: &[(&str, &[char])] = &[
    ("lower-greek", &LOWER_GREEK),
    ("hiragana", &HIRAGANA),
    ("hiragana-iroha", &HIRAGANA_IROHA),
    ("katakana", &KATAKANA),
    ("katakana-iroha", &KATAKANA_IROHA),
];

const LOWER_GREEK: [char; 24] = [
    '\u{3b1}', '\u{3b2}', '\u{3b3}', '\u{3b4}', '\u{3b5}', '\u{3b6}', '\u{3b7}', '\u{3b8}',
    '\u{3b9}', '\u{3ba}', '\u{3bb}', '\u{3bc}', '\u{3bd}', '\u{3be}', '\u{3bf}', '\u{3c0}',
    '\u{3c1}', '\u{3c3}', '\u{3c4}', '\u{3c5}', '\u{3c6}', '\u{3c7}', '\u{3c8}', '\u{3c9}',
]; // αβγδεζηθικλμνξοπρστυφχψω (ς, final sigma, deliberately absent per spec)

const HIRAGANA: [char; 48] = [
    '\u{3042}', '\u{3044}', '\u{3046}', '\u{3048}', '\u{304a}', '\u{304b}', '\u{304d}', '\u{304f}',
    '\u{3051}', '\u{3053}', '\u{3055}', '\u{3057}', '\u{3059}', '\u{305b}', '\u{305d}', '\u{305f}',
    '\u{3061}', '\u{3064}', '\u{3066}', '\u{3068}', '\u{306a}', '\u{306b}', '\u{306c}', '\u{306d}',
    '\u{306e}', '\u{306f}', '\u{3072}', '\u{3075}', '\u{3078}', '\u{307b}', '\u{307e}', '\u{307f}',
    '\u{3080}', '\u{3081}', '\u{3082}', '\u{3084}', '\u{3086}', '\u{3088}', '\u{3089}', '\u{308a}',
    '\u{308b}', '\u{308c}', '\u{308d}', '\u{308f}', '\u{3090}', '\u{3091}', '\u{3092}', '\u{3093}',
]; // あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわゐゑをん (dictionary order)

const HIRAGANA_IROHA: [char; 47] = [
    '\u{3044}', '\u{308d}', '\u{306f}', '\u{306b}', '\u{307b}', '\u{3078}', '\u{3068}', '\u{3061}',
    '\u{308a}', '\u{306c}', '\u{308b}', '\u{3092}', '\u{308f}', '\u{304b}', '\u{3088}', '\u{305f}',
    '\u{308c}', '\u{305d}', '\u{3064}', '\u{306d}', '\u{306a}', '\u{3089}', '\u{3080}', '\u{3046}',
    '\u{3090}', '\u{306e}', '\u{304a}', '\u{304f}', '\u{3084}', '\u{307e}', '\u{3051}', '\u{3075}',
    '\u{3053}', '\u{3048}', '\u{3066}', '\u{3042}', '\u{3055}', '\u{304d}', '\u{3086}', '\u{3081}',
    '\u{307f}', '\u{3057}', '\u{3091}', '\u{3072}', '\u{3082}', '\u{305b}', '\u{3059}',
]; // いろはにほへとちりぬるをわかよたれそつねならむうゐのおくやまけふこえてあさきゆめみしゑひもせす (iroha order)

const KATAKANA: [char; 48] = [
    '\u{30a2}', '\u{30a4}', '\u{30a6}', '\u{30a8}', '\u{30aa}', '\u{30ab}', '\u{30ad}', '\u{30af}',
    '\u{30b1}', '\u{30b3}', '\u{30b5}', '\u{30b7}', '\u{30b9}', '\u{30bb}', '\u{30bd}', '\u{30bf}',
    '\u{30c1}', '\u{30c4}', '\u{30c6}', '\u{30c8}', '\u{30ca}', '\u{30cb}', '\u{30cc}', '\u{30cd}',
    '\u{30ce}', '\u{30cf}', '\u{30d2}', '\u{30d5}', '\u{30d8}', '\u{30db}', '\u{30de}', '\u{30df}',
    '\u{30e0}', '\u{30e1}', '\u{30e2}', '\u{30e4}', '\u{30e6}', '\u{30e8}', '\u{30e9}', '\u{30ea}',
    '\u{30eb}', '\u{30ec}', '\u{30ed}', '\u{30ef}', '\u{30f0}', '\u{30f1}', '\u{30f2}', '\u{30f3}',
]; // アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヰヱヲン (dictionary order)

const KATAKANA_IROHA: [char; 47] = [
    '\u{30a4}', '\u{30ed}', '\u{30cf}', '\u{30cb}', '\u{30db}', '\u{30d8}', '\u{30c8}', '\u{30c1}',
    '\u{30ea}', '\u{30cc}', '\u{30eb}', '\u{30f2}', '\u{30ef}', '\u{30ab}', '\u{30e8}', '\u{30bf}',
    '\u{30ec}', '\u{30bd}', '\u{30c4}', '\u{30cd}', '\u{30ca}', '\u{30e9}', '\u{30e0}', '\u{30a6}',
    '\u{30f0}', '\u{30ce}', '\u{30aa}', '\u{30af}', '\u{30e4}', '\u{30de}', '\u{30b1}', '\u{30d5}',
    '\u{30b3}', '\u{30a8}', '\u{30c6}', '\u{30a2}', '\u{30b5}', '\u{30ad}', '\u{30e6}', '\u{30e1}',
    '\u{30df}', '\u{30b7}', '\u{30f1}', '\u{30d2}', '\u{30e2}', '\u{30bb}', '\u{30b9}',
]; // イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス (iroha order)

/// `alphabetic` system algorithm (bijective base-N), generalized from
/// [`format_alphabetic`] to take an arbitrary symbol table instead of a
/// contiguous ASCII range — needed because §6.2's non-Latin alphabets
/// aren't a `first_symbol + offset` sequence of codepoints. Not merged
/// into `format_alphabetic` itself (37n: sibling divergence noted):
/// `format_alphabetic` stays allocation-free per digit (`(first_symbol +
/// digit) as char`) for the two hottest names in this dispatcher
/// (`lower-alpha`/`upper-alpha`), while this version indexes a borrowed
/// slice — merging would mean allocating a 26-entry `Vec<char>` on every
/// `lower-alpha`/`upper-alpha` call just to share one loop body.
///
/// Same algorithm as [`format_alphabetic`]
/// <https://www.w3.org/TR/css-counter-styles-3/#alphabetic-system>: `None`
/// for `value < 1` (alphabetic's implicit range is strictly-positive
/// integers).
fn format_alphabetic_symbols(value: i32, symbols: &[char]) -> Option<String> {
    if value < 1 {
        return None;
    }
    let n = symbols.len() as i32;
    let mut remaining = value;
    let mut digits = Vec::new();
    while remaining != 0 {
        remaining -= 1;
        let digit = (remaining % n) as usize;
        digits.push(symbols[digit]);
        remaining /= n;
    }
    digits.reverse();
    Some(digits.into_iter().collect())
}

// ── §6.4 Fixed: `fixed`-system styles ───────────────────────────────────

/// §6.4 `fixed`-system predefined styles
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters>:
/// `cjk-earthly-branch` (12 symbols), `cjk-heavenly-stem` (10 symbols).
/// Neither `@counter-style` block specifies a first-symbol-value integer
/// after `system: fixed`, so both use the `fixed` system's default first
/// value of 1
/// <https://www.w3.org/TR/css-counter-styles-3/#fixed-system>.
const FIXED_STYLES: &[(&str, &[char])] = &[
    ("cjk-earthly-branch", &CJK_EARTHLY_BRANCH),
    ("cjk-heavenly-stem", &CJK_HEAVENLY_STEM),
];

const CJK_EARTHLY_BRANCH: [char; 12] = [
    '\u{5b50}', '\u{4e11}', '\u{5bc5}', '\u{536f}', '\u{8fb0}', '\u{5df3}', '\u{5348}', '\u{672a}',
    '\u{7533}', '\u{9149}', '\u{620c}', '\u{4ea5}',
]; // 子 丑 寅 卯 辰 巳 午 未 申 酉 戌 亥

const CJK_HEAVENLY_STEM: [char; 10] = [
    '\u{7532}', '\u{4e59}', '\u{4e19}', '\u{4e01}', '\u{620a}', '\u{5df1}', '\u{5e9a}', '\u{8f9b}',
    '\u{58ec}', '\u{7678}',
]; // 甲 乙 丙 丁 戊 己 庚 辛 壬 癸

/// `fixed` system algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#fixed-system>, specialized
/// to a first symbol value of 1 (see [`FIXED_STYLES`] doc): value `v` maps
/// to `symbols[v - 1]` for `v` in `1..=symbols.len()`; once the list is
/// exhausted the style "cannot represent" the value and — per the
/// algorithm text — falls back, hence `Option`. Neither
/// `cjk-earthly-branch` nor `cjk-heavenly-stem` uses a negative sign
/// (`fixed` isn't in the negative descriptor's "uses a negative sign" list
/// <https://www.w3.org/TR/css-counter-styles-3/#descdef-counter-style-negative>),
/// so out-of-range negative/zero values fall straight into this same
/// bounds check rather than needing separate handling.
fn format_fixed(value: i32, symbols: &[char]) -> Option<String> {
    if value < 1 || value as usize > symbols.len() {
        return None;
    }
    Some(symbols[(value - 1) as usize].to_string())
}

// ── §7.1 Longhand East Asian: Japanese / Korean `additive`-system styles ─

/// §7.1 Longhand East Asian `additive`-system styles
/// <https://www.w3.org/TR/css-counter-styles-3/#complex-predefined-counters>:
/// `japanese-informal`, `japanese-formal`, `korean-hangul-formal`,
/// `korean-hanja-informal`, `korean-hanja-formal`. Each entry is `(name,
/// additive-symbols table, negative prefix)`; all five share `range: -9999
/// 9999` (§7.1's opening paragraph — see the `LONGHAND_ADDITIVE_STYLES`
/// dispatch arm in [`format_named_counter`] for the fallback-chain
/// rationale) and are run through [`format_longhand_additive`], not
/// [`format_additive_named`], because — unlike every §6.1 additive style —
/// their range includes 0 and negative values, so generate-a-counter's
/// "use absolute value, then wrap in the negative sign" steps actually
/// apply here.
///
/// Codepoints transcribed from the normative `@counter-style` stylesheet
/// fragment in §7.1.1/§7.1.2; cross-checked against the 10-column
/// reference table earlier in §7.1 (values 0, 1, 2, 3, 10, 11, 99, 100,
/// 101, 6001 given for all nine §7.1 styles) — e.g. `japanese-informal`
/// (6001) = weight-6000 + weight-1 = 六千 + 一 = "六千一", matching the
/// table's `japanese-informal` column exactly (no zero-digit marker,
/// unlike the digit-marker Chinese styles at the same value — see
/// [`CHINESE_DIGIT_MARKER_STYLES`]).
const LONGHAND_ADDITIVE_STYLES: &[(&str, AdditiveSymbols, &str)] = &[
    (
        "japanese-informal",
        &JAPANESE_INFORMAL_ADDITIVE,
        JAPANESE_NEGATIVE,
    ),
    (
        "japanese-formal",
        &JAPANESE_FORMAL_ADDITIVE,
        JAPANESE_NEGATIVE,
    ),
    (
        "korean-hangul-formal",
        &KOREAN_HANGUL_FORMAL_ADDITIVE,
        KOREAN_NEGATIVE,
    ),
    (
        "korean-hanja-informal",
        &KOREAN_HANJA_INFORMAL_ADDITIVE,
        KOREAN_NEGATIVE,
    ),
    (
        "korean-hanja-formal",
        &KOREAN_HANJA_FORMAL_ADDITIVE,
        KOREAN_NEGATIVE,
    ),
];

/// `negative: "\30DE\30A4\30CA\30B9"` (マイナス) — identical string shared
/// verbatim by both `japanese-informal` and `japanese-formal`'s
/// `@counter-style` blocks.
const JAPANESE_NEGATIVE: &str = "\u{30de}\u{30a4}\u{30ca}\u{30b9}";

/// `negative: "\B9C8\C774\B108\C2A4  "` (마이너스, followed by **two**
/// literal U+0020 spaces inside the quoted string — confirmed against the
/// raw stylesheet text; the informative comment beside it says "followed
/// by a space" (singular), but the normative descriptor value itself has
/// two) — identical string shared verbatim by all three Korean
/// `@counter-style` blocks.
const KOREAN_NEGATIVE: &str = "\u{b9c8}\u{c774}\u{b108}\u{c2a4}  ";

const JAPANESE_INFORMAL_ADDITIVE: [(i32, &str); 37] = [
    (9000, "\u{4e5d}\u{5343}"),
    (8000, "\u{516b}\u{5343}"),
    (7000, "\u{4e03}\u{5343}"),
    (6000, "\u{516d}\u{5343}"),
    (5000, "\u{4e94}\u{5343}"),
    (4000, "\u{56db}\u{5343}"),
    (3000, "\u{4e09}\u{5343}"),
    (2000, "\u{4e8c}\u{5343}"),
    (1000, "\u{5343}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{4e09}\u{767e}"),
    (200, "\u{4e8c}\u{767e}"),
    (100, "\u{767e}"),
    (90, "\u{4e5d}\u{5341}"),
    (80, "\u{516b}\u{5341}"),
    (70, "\u{4e03}\u{5341}"),
    (60, "\u{516d}\u{5341}"),
    (50, "\u{4e94}\u{5341}"),
    (40, "\u{56db}\u{5341}"),
    (30, "\u{4e09}\u{5341}"),
    (20, "\u{4e8c}\u{5341}"),
    (10, "\u{5341}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{4e09}"),
    (2, "\u{4e8c}"),
    (1, "\u{4e00}"),
    (0, "\u{3007}"),
];

const JAPANESE_FORMAL_ADDITIVE: [(i32, &str); 37] = [
    (9000, "\u{4e5d}\u{9621}"),
    (8000, "\u{516b}\u{9621}"),
    (7000, "\u{4e03}\u{9621}"),
    (6000, "\u{516d}\u{9621}"),
    (5000, "\u{4f0d}\u{9621}"),
    (4000, "\u{56db}\u{9621}"),
    (3000, "\u{53c2}\u{9621}"),
    (2000, "\u{5f10}\u{9621}"),
    (1000, "\u{58f1}\u{9621}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4f0d}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{53c2}\u{767e}"),
    (200, "\u{5f10}\u{767e}"),
    (100, "\u{58f1}\u{767e}"),
    (90, "\u{4e5d}\u{62fe}"),
    (80, "\u{516b}\u{62fe}"),
    (70, "\u{4e03}\u{62fe}"),
    (60, "\u{516d}\u{62fe}"),
    (50, "\u{4f0d}\u{62fe}"),
    (40, "\u{56db}\u{62fe}"),
    (30, "\u{53c2}\u{62fe}"),
    (20, "\u{5f10}\u{62fe}"),
    (10, "\u{58f1}\u{62fe}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4f0d}"),
    (4, "\u{56db}"),
    (3, "\u{53c2}"),
    (2, "\u{5f10}"),
    (1, "\u{58f1}"),
    (0, "\u{96f6}"),
];

const KOREAN_HANGUL_FORMAL_ADDITIVE: [(i32, &str); 37] = [
    (9000, "\u{ad6c}\u{cc9c}"),
    (8000, "\u{d314}\u{cc9c}"),
    (7000, "\u{ce60}\u{cc9c}"),
    (6000, "\u{c721}\u{cc9c}"),
    (5000, "\u{c624}\u{cc9c}"),
    (4000, "\u{c0ac}\u{cc9c}"),
    (3000, "\u{c0bc}\u{cc9c}"),
    (2000, "\u{c774}\u{cc9c}"),
    (1000, "\u{c77c}\u{cc9c}"),
    (900, "\u{ad6c}\u{bc31}"),
    (800, "\u{d314}\u{bc31}"),
    (700, "\u{ce60}\u{bc31}"),
    (600, "\u{c721}\u{bc31}"),
    (500, "\u{c624}\u{bc31}"),
    (400, "\u{c0ac}\u{bc31}"),
    (300, "\u{c0bc}\u{bc31}"),
    (200, "\u{c774}\u{bc31}"),
    (100, "\u{c77c}\u{bc31}"),
    (90, "\u{ad6c}\u{c2ed}"),
    (80, "\u{d314}\u{c2ed}"),
    (70, "\u{ce60}\u{c2ed}"),
    (60, "\u{c721}\u{c2ed}"),
    (50, "\u{c624}\u{c2ed}"),
    (40, "\u{c0ac}\u{c2ed}"),
    (30, "\u{c0bc}\u{c2ed}"),
    (20, "\u{c774}\u{c2ed}"),
    (10, "\u{c77c}\u{c2ed}"),
    (9, "\u{ad6c}"),
    (8, "\u{d314}"),
    (7, "\u{ce60}"),
    (6, "\u{c721}"),
    (5, "\u{c624}"),
    (4, "\u{c0ac}"),
    (3, "\u{c0bc}"),
    (2, "\u{c774}"),
    (1, "\u{c77c}"),
    (0, "\u{c601}"),
];

const KOREAN_HANJA_INFORMAL_ADDITIVE: [(i32, &str); 37] = [
    (9000, "\u{4e5d}\u{5343}"),
    (8000, "\u{516b}\u{5343}"),
    (7000, "\u{4e03}\u{5343}"),
    (6000, "\u{516d}\u{5343}"),
    (5000, "\u{4e94}\u{5343}"),
    (4000, "\u{56db}\u{5343}"),
    (3000, "\u{4e09}\u{5343}"),
    (2000, "\u{4e8c}\u{5343}"),
    (1000, "\u{5343}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{4e09}\u{767e}"),
    (200, "\u{4e8c}\u{767e}"),
    (100, "\u{767e}"),
    (90, "\u{4e5d}\u{5341}"),
    (80, "\u{516b}\u{5341}"),
    (70, "\u{4e03}\u{5341}"),
    (60, "\u{516d}\u{5341}"),
    (50, "\u{4e94}\u{5341}"),
    (40, "\u{56db}\u{5341}"),
    (30, "\u{4e09}\u{5341}"),
    (20, "\u{4e8c}\u{5341}"),
    (10, "\u{5341}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{4e09}"),
    (2, "\u{4e8c}"),
    (1, "\u{4e00}"),
    (0, "\u{96f6}"),
];

const KOREAN_HANJA_FORMAL_ADDITIVE: [(i32, &str); 37] = [
    (9000, "\u{4e5d}\u{4edf}"),
    (8000, "\u{516b}\u{4edf}"),
    (7000, "\u{4e03}\u{4edf}"),
    (6000, "\u{516d}\u{4edf}"),
    (5000, "\u{4e94}\u{4edf}"),
    (4000, "\u{56db}\u{4edf}"),
    (3000, "\u{53c3}\u{4edf}"),
    (2000, "\u{8cb3}\u{4edf}"),
    (1000, "\u{58f9}\u{4edf}"),
    (900, "\u{4e5d}\u{767e}"),
    (800, "\u{516b}\u{767e}"),
    (700, "\u{4e03}\u{767e}"),
    (600, "\u{516d}\u{767e}"),
    (500, "\u{4e94}\u{767e}"),
    (400, "\u{56db}\u{767e}"),
    (300, "\u{53c3}\u{767e}"),
    (200, "\u{8cb3}\u{767e}"),
    (100, "\u{58f9}\u{767e}"),
    (90, "\u{4e5d}\u{62fe}"),
    (80, "\u{516b}\u{62fe}"),
    (70, "\u{4e03}\u{62fe}"),
    (60, "\u{516d}\u{62fe}"),
    (50, "\u{4e94}\u{62fe}"),
    (40, "\u{56db}\u{62fe}"),
    (30, "\u{53c3}\u{62fe}"),
    (20, "\u{8cb3}\u{62fe}"),
    (10, "\u{58f9}\u{62fe}"),
    (9, "\u{4e5d}"),
    (8, "\u{516b}"),
    (7, "\u{4e03}"),
    (6, "\u{516d}"),
    (5, "\u{4e94}"),
    (4, "\u{56db}"),
    (3, "\u{53c3}"),
    (2, "\u{8cb3}"),
    (1, "\u{58f9}"),
    (0, "\u{96f6}"),
];

/// Range-check, absolute-value substitution, and negative-sign wrap for
/// [`LONGHAND_ADDITIVE_STYLES`] entries — the generate-a-counter steps
/// [`format_additive_named`]'s §6.1 callers don't need (their ranges
/// exclude 0/negative outright). `value.unsigned_abs()` (never `.abs()`,
/// which panics on `i32::MIN` in debug builds — same reasoning as
/// [`format_decimal_leading_zero`]) always fits back into `i32` here since
/// the range check above already bounds `|value| <= 9999`.
fn format_longhand_additive(value: i32, table: AdditiveSymbols, negative: &str) -> Option<String> {
    if !(-9999..=9999).contains(&value) {
        return None;
    }
    let magnitude = value.unsigned_abs() as i32;
    let body = format_additive_system(magnitude, table)?;
    if value < 0 {
        Some(format!("{negative}{body}"))
    } else {
        Some(body)
    }
}

// ── §7.1.3 Longhand East Asian: Chinese digit-marker styles ─────────────

/// Per-style character table for the §7.1.3 Chinese digit-marker algorithm
/// ([`format_chinese_digit_marker`]) — `digits[d]` is the glyph for
/// decimal digit `d` (`digits[0]` is 零 U+96F6 for all four styles),
/// `tens`/`hundreds`/`thousands` are the positional markers, `negative` is
/// the single-symbol negative prefix, and `informal` selects step 3 of the
/// algorithm (only the informal styles drop the leading tens-digit glyph
/// for magnitudes 10-19).
struct ChineseDigitTable {
    digits: [char; 10],
    tens: char,
    hundreds: char,
    thousands: char,
    negative: char,
    informal: bool,
}

/// §7.1.3 Chinese digit-marker predefined styles
/// <https://www.w3.org/TR/css-counter-styles-3/#complex-predefined-counters>:
/// `simp-chinese-informal`, `simp-chinese-formal`, `trad-chinese-informal`,
/// `trad-chinese-formal`, and `cjk-ideographic` (spec's own §7.1.3 dfn:
/// "identical to trad-chinese-informal. (It exists for legacy reasons.)" —
/// hence sharing `&TRAD_CHINESE_INFORMAL` by reference rather than a
/// separate table, same pattern as `khmer`/`cambodian` above). Character
/// table transcribed from the spec's own "Values / Codepoints" table in
/// §7.1.3.
const CHINESE_DIGIT_MARKER_STYLES: &[(&str, &ChineseDigitTable)] = &[
    ("simp-chinese-informal", &SIMP_CHINESE_INFORMAL),
    ("simp-chinese-formal", &SIMP_CHINESE_FORMAL),
    ("trad-chinese-informal", &TRAD_CHINESE_INFORMAL),
    ("trad-chinese-formal", &TRAD_CHINESE_FORMAL),
    ("cjk-ideographic", &TRAD_CHINESE_INFORMAL),
];

const SIMP_CHINESE_INFORMAL: ChineseDigitTable = ChineseDigitTable {
    digits: [
        '\u{96f6}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ], // 零一二三四五六七八九
    tens: '\u{5341}',      // 十
    hundreds: '\u{767e}',  // 百
    thousands: '\u{5343}', // 千
    negative: '\u{8d1f}',  // 负
    informal: true,
};

const SIMP_CHINESE_FORMAL: ChineseDigitTable = ChineseDigitTable {
    digits: [
        '\u{96f6}', '\u{58f9}', '\u{8d30}', '\u{53c1}', '\u{8086}', '\u{4f0d}', '\u{9646}',
        '\u{67d2}', '\u{634c}', '\u{7396}',
    ], // 零壹贰叁肆伍陆柒捌玖
    tens: '\u{62fe}',      // 拾
    hundreds: '\u{4f70}',  // 佰
    thousands: '\u{4edf}', // 仟
    negative: '\u{8d1f}',  // 负
    informal: false,
};

const TRAD_CHINESE_INFORMAL: ChineseDigitTable = ChineseDigitTable {
    digits: [
        '\u{96f6}', '\u{4e00}', '\u{4e8c}', '\u{4e09}', '\u{56db}', '\u{4e94}', '\u{516d}',
        '\u{4e03}', '\u{516b}', '\u{4e5d}',
    ], // 零一二三四五六七八九
    tens: '\u{5341}',      // 十
    hundreds: '\u{767e}',  // 百
    thousands: '\u{5343}', // 千
    negative: '\u{8ca0}',  // 負
    informal: true,
};

const TRAD_CHINESE_FORMAL: ChineseDigitTable = ChineseDigitTable {
    digits: [
        '\u{96f6}', '\u{58f9}', '\u{8cb3}', '\u{53c3}', '\u{8086}', '\u{4f0d}', '\u{9678}',
        '\u{67d2}', '\u{634c}', '\u{7396}',
    ], // 零壹貳參肆伍陸柒捌玖
    tens: '\u{62fe}',      // 拾
    hundreds: '\u{4f70}',  // 佰
    thousands: '\u{4edf}', // 仟
    negative: '\u{8ca0}',  // 負
    informal: false,
};

/// §7.1.3 Chinese longhand digit-marker algorithm
/// <https://www.w3.org/TR/css-counter-styles-3/#complex-predefined-counters>
/// for `simp-chinese-informal`, `simp-chinese-formal`,
/// `trad-chinese-informal`, `trad-chinese-formal`, and `cjk-ideographic`
/// (dispatches to the same `&TRAD_CHINESE_INFORMAL` table as
/// `trad-chinese-informal` — see [`CHINESE_DIGIT_MARKER_STYLES`]) — a
/// genuinely different
/// algorithm from [`format_additive_system`], not just a different table:
/// unlike the §7.1 Japanese/Korean `additive`-system styles (which just
/// skip zero-weight positions silently, e.g. 6001 → 六千一, no zero
/// digit), the Chinese styles explicitly render an interior zero run as a
/// single collapsed zero glyph (6001 → 六千零一) — verified against the
/// §7.1 ten-column reference table's `6001`/`101` rows for all four
/// styles, and against the full first-120-values worked table given for
/// `simp-chinese-informal` (spot-checked at the 10-19 informal
/// tens-digit-removal boundary, 20/21, and 100/101/110/111 — the "remove
/// tens digit for 10-19" special case (algorithm step 3) does NOT extend
/// to 110/111, which keep their leading 一/壹).
///
/// Implements the prose algorithm's five steps working on ASCII digit
/// placeholders (only substituted to the target script in the final step,
/// per the algorithm's own ordering — "Replace the digits 0-9 ... Return
/// the resultant string" is explicitly the last step):
/// 1. `magnitude == 0` short-circuits to the style's zero glyph.
/// 2. Build "digit + marker" per decimal position (thousands/hundreds/tens
///    get a marker only when their digit is non-zero; ones never gets a
///    marker) by walking `magnitude.to_string()` MSB-first.
/// 3. Informal styles only: for `magnitude` in `10..=19`, drop the leading
///    tens-digit placeholder but keep its marker (`repr.remove(0)` — the
///    leading char is always the ASCII tens digit at byte offset 0, a
///    valid char boundary regardless of what multi-byte marker glyphs
///    follow).
/// 4. Drop a trailing run of `'0'` placeholders, then collapse any
///    remaining (interior) run of `'0'` placeholders to a single `'0'`.
/// 5. Substitute remaining ASCII `'0'..='9'` placeholders with
///    `table.digits`; marker/negative characters were already final.
///
/// `-9999..=9999` range and negative-sign handling mirror
/// [`format_longhand_additive`] (§7.1's opening paragraph covers both
/// families) — `None` outside that range.
fn format_chinese_digit_marker(value: i32, table: &ChineseDigitTable) -> Option<String> {
    if !(-9999..=9999).contains(&value) {
        return None;
    }
    let magnitude = value.unsigned_abs();
    if magnitude == 0 {
        return Some(table.digits[0].to_string());
    }

    // Step 2.
    let decimal = magnitude.to_string();
    let place_count = decimal.len();
    let mut repr = String::new();
    for (i, ch) in decimal.chars().enumerate() {
        repr.push(ch);
        if ch != '0' {
            match place_count - 1 - i {
                1 => repr.push(table.tens),
                2 => repr.push(table.hundreds),
                3 => repr.push(table.thousands),
                // Ones place (0) never gets a marker; anything beyond
                // thousands is unreachable — the range check above bounds
                // `magnitude` to at most 4 decimal digits.
                _ => {}
            }
        }
    }

    // Step 3 (informal only).
    if table.informal && (10..=19).contains(&magnitude) {
        repr.remove(0);
    }

    // Step 4.
    while repr.ends_with('0') {
        repr.pop();
    }
    let mut collapsed = String::with_capacity(repr.len());
    let mut prev_was_zero = false;
    for ch in repr.chars() {
        if ch == '0' {
            if !prev_was_zero {
                collapsed.push('0');
            }
            prev_was_zero = true;
        } else {
            collapsed.push(ch);
            prev_was_zero = false;
        }
    }

    // Step 5.
    let mut out = String::with_capacity(collapsed.len());
    for ch in collapsed.chars() {
        match ch.to_digit(10) {
            Some(d) => out.push(table.digits[d as usize]),
            None => out.push(ch),
        }
    }

    if value < 0 {
        Some(format!("{}{out}", table.negative))
    } else {
        Some(out)
    }
}

// ── §7.2 Ethiopic Numeric Counter Style ─────────────────────────────────

/// Units 1-9 (`ETHIOPIC_UNITS[n - 1]` = glyph for `n`).
const ETHIOPIC_UNITS: [char; 9] = [
    '\u{1369}', '\u{136a}', '\u{136b}', '\u{136c}', '\u{136d}', '\u{136e}', '\u{136f}', '\u{1370}',
    '\u{1371}',
]; // ፩ ፪ ፫ ፬ ፭ ፮ ፯ ፰ ፱

/// Tens 10-90 (`ETHIOPIC_TENS[n - 1]` = glyph for `10n`).
const ETHIOPIC_TENS: [char; 9] = [
    '\u{1372}', '\u{1373}', '\u{1374}', '\u{1375}', '\u{1376}', '\u{1377}', '\u{1378}', '\u{1379}',
    '\u{137a}',
]; // ፲ ፳ ፴ ፵ ፶ ፷ ፸ ፹ ፺

const ETHIOPIC_HUNDRED: char = '\u{137b}'; // ፻ (hundred group separator)
const ETHIOPIC_TEN_THOUSAND: char = '\u{137c}'; // ፼ (ten-thousand group separator)

/// §7.2 Ethiopic Numeric Counter Style
/// <https://www.w3.org/TR/css-counter-styles-3/#ethiopic-numeric-counter-style>:
/// `ethiopic-numeric`. A bespoke base-100-grouped algorithm (not
/// `additive`, not `numeric`) — implements the spec's six-step prose
/// algorithm directly.
///
/// Range is `1 infinite`; `value == 1` is a special-cased return per step
/// 1 of the algorithm text.
///
/// **Verified against all three of the spec's own worked examples** (100 →
/// ፻; 78010092 → ፸፰፻፩፼፺፪; 780100000092 → ፸፰፻፩፼፼፺፪ — the third one is
/// load-bearing, see below). The two group-separator steps (6 and 7) are
/// asymmetric in a way that's easy to misread on a first pass: step 6 (፻,
/// odd-index groups) explicitly excepts groups whose original value was
/// zero ("except groups which originally had a value of zero"); step 7
/// (፼, even-index groups except index 0) has **no** such exception — an
/// even-index group with value 0 still gets a bare ፼ (no digit before it,
/// since the digit itself is suppressed by step 4's "group has value
/// zero" clause, but the separator is not). The 780100000092 example is
/// the only one of the three that exercises this: its group 2 (value 0,
/// even index) contributes a bare ፼, producing the doubled ፼፼ in the
/// expected output; treating step 7 as sharing step 6's zero-value
/// exception (the natural first read) produces a single ፼ and silently
/// drops the doubled-separator case.
fn format_ethiopic_numeric(value: i32) -> Option<String> {
    if value < 1 {
        return None;
    }
    if value == 1 {
        return Some(ETHIOPIC_UNITS[0].to_string());
    }

    // Steps 2/3: base-100 groups, least-significant first (index 0).
    let mut groups = Vec::new();
    let mut remaining = value;
    while remaining > 0 {
        groups.push(remaining % 100);
        remaining /= 100;
    }
    let msb_index = groups.len() - 1;

    let mut out = String::new();
    for i in (0..groups.len()).rev() {
        let group_value = groups[i];
        // Step 4: suppress the digit(s) when the group is zero, is the
        // most-significant group with value 1, or has an odd index with
        // value 1.
        let suppress_digit = group_value == 0
            || (i == msb_index && group_value == 1)
            || (i % 2 == 1 && group_value == 1);
        if !suppress_digit {
            let tens = group_value / 10;
            let units = group_value % 10;
            if tens > 0 {
                out.push(ETHIOPIC_TENS[(tens - 1) as usize]);
            }
            if units > 0 {
                out.push(ETHIOPIC_UNITS[(units - 1) as usize]);
            }
        }
        // Steps 6/7 — see the asymmetry note on this function's doc.
        if i % 2 == 1 {
            if group_value != 0 {
                out.push(ETHIOPIC_HUNDRED);
            }
        } else if i != 0 {
            out.push(ETHIOPIC_TEN_THOUSAND);
        }
    }
    Some(out)
}

/// Join a nested counter stack with `separator`, formatting each level via
/// [`format_counter`].
///
/// **Visibility**: `pub(crate)` (not private) —
/// sibling `page::context` module needs both this and [`format_counter`] to
/// fully resolve a `string-set` content-list's `counter()`/`counters()`
/// items (design §7.2's `NamedStringState`, which — unlike this crate's
/// earlier placeholder — needs real text, not a deferred snapshot).
/// `pub(crate)` rather than full `pub`: no cross-crate caller needs either
/// helper directly (raikiri-dom only calls through
/// [`crate::page::PageContext::apply_directive`]), so crate-internal
/// visibility is the minimal fix (resolved here rather than duplicating the
/// formatting logic elsewhere).
pub(crate) fn join_counter_stack(stack: &[i32], separator: &str, style: &CounterStyle) -> String {
    stack
        .iter()
        .map(|v| format_counter(*v, style))
        .collect::<Vec<_>>()
        .join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    use smol_str::SmolStr;

    // ── Skeleton pins ─────────

    #[test]
    fn target_registry_default_is_empty() {
        // Constructibility pin: `TargetRegistry::default()` yields an empty
        // registry. The directive-apply driver will consume this
        // constructor.
        let reg = TargetRegistry::default();
        assert!(reg.resolved.is_empty());
        assert!(reg.pending_slots.is_empty());
        assert_eq!(reg.next_sequence, 0);
        assert_eq!(reg.page_index, 0);
    }

    #[test]
    fn target_registry_shape_matches_design_7_2() {
        // Canonical shape pin (design §7.2, lines 1961-1964):
        //   resolved: HashMap<Symbol, TargetInfo>
        //   pending_slots: Vec<TargetSlot>
        // If this test breaks, the type has drifted from the design and
        // that must be reconciled before landing follow-up work.
        // `TargetSlot.id` is the design's `TargetSlotId` (§7.4 line 2098 /
        // §7.6 lines 2312-2317, Finding #4) —
        // a break here means that pairing drifted, not just the two
        // enumerated fields above.
        let mut reg = TargetRegistry::default();
        reg.resolved
            .insert(Symbol::new("fragment-1"), TargetInfo::default());
        reg.pending_slots.push(TargetSlot {
            id: TargetSlotId {
                page_index: 0,
                sequence: 0,
            },
            fragment_id: Symbol::new("fragment-1"),
            request: TargetRequest::Text {
                part: ContentPart::Content,
            },
        });
        assert_eq!(reg.resolved.len(), 1);
        assert_eq!(reg.pending_slots.len(), 1);
        assert!(reg.resolved.contains_key(&Symbol::new("fragment-1")));
    }

    // ── Fragment parse ──────────────────────────────────────────────

    #[test]
    fn parse_fragment_accepts_hash_prefixed_ident() {
        assert_eq!(parse_fragment("#chapter-3"), Some("chapter-3"));
        assert_eq!(parse_fragment("#a"), Some("a"));
    }

    #[test]
    fn parse_fragment_rejects_bare_ident_and_external_url() {
        assert_eq!(parse_fragment("chapter-3"), None);
        assert_eq!(parse_fragment("https://example.com/#x"), None);
        assert_eq!(parse_fragment(""), None);
    }

    #[test]
    fn parse_fragment_rejects_empty_fragment() {
        // `#` alone: `strip_prefix` succeeds but leaves `""` — reject so
        // downstream doesn't lookup Symbol::new("") in resolved.
        assert_eq!(parse_fragment("#"), None);
    }

    // ── Counter formatting ──────────────────────────────────────────

    #[test]
    fn format_counter_decimal() {
        assert_eq!(format_counter(0, &CounterStyle::Decimal), "0");
        assert_eq!(format_counter(1, &CounterStyle::Decimal), "1");
        assert_eq!(format_counter(42, &CounterStyle::Decimal), "42");
        assert_eq!(format_counter(-3, &CounterStyle::Decimal), "-3");
    }

    #[test]
    fn format_counter_named_unknown_falls_back_to_decimal() {
        // generate-a-counter step 1 <https://www.w3.org/TR/css-counter-styles-3/#generate-a-counter>:
        // "If the counter style is unknown, ... generate a counter
        // representation using the decimal style." Also covers the
        // registry-dependent path (custom @counter-style names — og2
        // comment 2026-07-27 scope (b), unreachable until raikiri-style
        // grows a registry): both an unrecognized keyword and a
        // hypothetical custom name land here identically.
        let style = CounterStyle::Named(SmolStr::new("my-custom-style"));
        assert_eq!(format_counter(4, &style), "4");
    }

    #[test]
    fn format_counter_decimal_leading_zero() {
        let style = CounterStyle::Named(SmolStr::new("decimal-leading-zero"));
        assert_eq!(format_counter(0, &style), "00");
        assert_eq!(format_counter(5, &style), "05");
        assert_eq!(format_counter(15, &style), "15");
        assert_eq!(format_counter(-5, &style), "-05");
        assert_eq!(format_counter(-100, &style), "-100");
    }

    #[test]
    fn format_counter_lower_roman() {
        let style = CounterStyle::Named(SmolStr::new("lower-roman"));
        assert_eq!(format_counter(1, &style), "i");
        assert_eq!(format_counter(4, &style), "iv"); // additive `continue` path (skips `v`)
        assert_eq!(format_counter(1000, &style), "m"); // single-tuple exact `break` path
        assert_eq!(format_counter(3000, &style), "mmm"); // same-weight multi-rep
        assert_eq!(format_counter(3999, &style), "mmmcmxcix"); // full additive-symbols sweep
    }

    #[test]
    fn format_counter_upper_roman() {
        let style = CounterStyle::Named(SmolStr::new("upper-roman"));
        assert_eq!(format_counter(4, &style), "IV");
        assert_eq!(format_counter(3999, &style), "MMMCMXCIX");
    }

    #[test]
    fn format_counter_roman_out_of_range_falls_back_to_decimal() {
        // `range: 1 3999` <https://www.w3.org/TR/css-counter-styles-3/#simple-numeric>:
        // 0, negative, and >3999 are all out of range — fallback renders the
        // *original* signed value as decimal (generate-a-counter step 2
        // uses "the same counter value", not the absolute value).
        let style = CounterStyle::Named(SmolStr::new("lower-roman"));
        assert_eq!(format_counter(0, &style), "0");
        assert_eq!(format_counter(-1, &style), "-1");
        assert_eq!(format_counter(4000, &style), "4000");
    }

    #[test]
    fn format_counter_roman_is_case_insensitive() {
        let style = CounterStyle::Named(SmolStr::new("UPPER-ROMAN"));
        assert_eq!(format_counter(4, &style), "IV");
    }

    #[test]
    fn format_counter_lower_alpha() {
        let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
        assert_eq!(format_counter(1, &style), "a");
        assert_eq!(format_counter(26, &style), "z");
        assert_eq!(format_counter(27, &style), "aa"); // bijective-base-26 wrap
        assert_eq!(format_counter(52, &style), "az");
    }

    #[test]
    fn format_counter_upper_alpha() {
        let style = CounterStyle::Named(SmolStr::new("upper-alpha"));
        assert_eq!(format_counter(1, &style), "A");
        assert_eq!(format_counter(28, &style), "AB");
    }

    #[test]
    fn format_counter_alpha_out_of_range_falls_back_to_decimal() {
        // Alphabetic system range defaults to strictly-positive integers
        // <https://www.w3.org/TR/css-counter-styles-3/#counter-style-range>.
        let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
        assert_eq!(format_counter(0, &style), "0");
        assert_eq!(format_counter(-1, &style), "-1");
    }

    #[test]
    fn format_counter_latin_aliases_match_alpha() {
        // §6.2 defines lower-latin / upper-latin with symbol tables
        // identical to lower-alpha / upper-alpha
        // <https://www.w3.org/TR/css-counter-styles-3/#simple-alphabetic>.
        let lower = CounterStyle::Named(SmolStr::new("lower-latin"));
        let upper = CounterStyle::Named(SmolStr::new("upper-latin"));
        assert_eq!(format_counter(27, &lower), "aa");
        assert_eq!(format_counter(27, &upper), "AA");
    }

    #[test]
    fn format_counter_disc_circle_square_are_value_independent() {
        // `system: cyclic` with a single symbol always renders that symbol,
        // for any value (range -infinity..infinity, no fallback path) —
        // and never the `suffix: " "` from the §6.3 block (see
        // format_named_counter doc: prefix/suffix are a ::marker concern,
        // not part of the counter()/target-counter() string).
        let disc = CounterStyle::Named(SmolStr::new("disc"));
        let circle = CounterStyle::Named(SmolStr::new("circle"));
        let square = CounterStyle::Named(SmolStr::new("square"));
        assert_eq!(format_counter(1, &disc), "\u{2022}");
        assert_eq!(format_counter(-7, &disc), "\u{2022}");
        assert_eq!(format_counter(1, &circle), "\u{25E6}");
        assert_eq!(format_counter(1, &square), "\u{25AA}");
    }

    /// Test-only shorthand for `CounterStyle::Named(SmolStr::new(name))` —
    /// introduced here (37n: sibling divergence noted) because the ce3k
    /// additions below construct it ~60 times across 42 new styles; every
    /// pre-existing test above keeps the inline form untouched.
    fn named(name: &str) -> CounterStyle {
        CounterStyle::Named(SmolStr::new(name))
    }

    // ── §6.1 Numeric: `numeric`-system digit-table styles ────────────

    #[test]
    fn format_counter_numeric_digit_styles_value_one() {
        // Cheap transcription-error catcher across all 18 numeric-digit
        // names (17 distinct tables + the `khmer` alias): value 1 must be
        // each table's `digits[1]` entry, per the dfn line's own first
        // `(e.g., ...)` example.
        for (name, first) in [
            ("arabic-indic", '\u{661}'),
            ("bengali", '\u{9e7}'),
            ("cambodian", '\u{17e1}'),
            ("khmer", '\u{17e1}'),
            ("devanagari", '\u{967}'),
            ("gujarati", '\u{ae7}'),
            ("gurmukhi", '\u{a67}'),
            ("kannada", '\u{ce7}'),
            ("lao", '\u{ed1}'),
            ("malayalam", '\u{d67}'),
            ("mongolian", '\u{1811}'),
            ("myanmar", '\u{1041}'),
            ("oriya", '\u{b67}'),
            ("persian", '\u{6f1}'),
            ("tamil", '\u{be7}'),
            ("telugu", '\u{c67}'),
            ("thai", '\u{e51}'),
            ("tibetan", '\u{f21}'),
        ] {
            assert_eq!(
                format_counter(1, &named(name)),
                first.to_string(),
                "style {name} at value 1"
            );
        }
    }

    #[test]
    fn format_counter_numeric_digit_styles_multi_digit_reference_triple() {
        // Each dfn line's `(e.g., ..., 98, 99, 100)` reference triple,
        // spot-checked on 3 of the 18 tables (arabic-indic, bengali,
        // tibetan) — exercises the positional two/three-digit path, not
        // just the single-digit case above.
        assert_eq!(
            format_counter(98, &named("arabic-indic")),
            "\u{669}\u{668}" // ٩٨
        );
        assert_eq!(
            format_counter(99, &named("arabic-indic")),
            "\u{669}\u{669}" // ٩٩
        );
        assert_eq!(
            format_counter(100, &named("arabic-indic")),
            "\u{661}\u{660}\u{660}" // ١٠٠
        );
        assert_eq!(format_counter(98, &named("bengali")), "\u{9ef}\u{9ee}"); // ৯৮
        assert_eq!(
            format_counter(100, &named("bengali")),
            "\u{9e7}\u{9e6}\u{9e6}" // ১০০
        );
        assert_eq!(
            format_counter(100, &named("tibetan")),
            "\u{f21}\u{f20}\u{f20}" // ༡༠༠
        );
    }

    #[test]
    fn format_counter_numeric_digit_styles_negative_uses_ascii_hyphen() {
        // `numeric` system default `negative: "-"` — none of these
        // @counter-style blocks override it.
        assert_eq!(format_counter(-1, &named("arabic-indic")), "-\u{661}");
        assert_eq!(format_counter(0, &named("bengali")), "\u{9e6}");
    }

    #[test]
    fn format_counter_cjk_decimal_is_positional_not_longhand() {
        // §6.1 dfn: "(e.g., 一, 二, 三, ..., 九八, 九九, 一〇〇)" — digit
        // substitution, not the longhand-additive algorithm §7.1 uses for
        // otherwise-similar Han-numeral styles.
        assert_eq!(format_counter(1, &named("cjk-decimal")), "\u{4e00}"); // 一
        assert_eq!(format_counter(2, &named("cjk-decimal")), "\u{4e8c}"); // 二
        assert_eq!(
            format_counter(98, &named("cjk-decimal")),
            "\u{4e5d}\u{516b}" // 九八
        );
        assert_eq!(
            format_counter(100, &named("cjk-decimal")),
            "\u{4e00}\u{3007}\u{3007}" // 一〇〇
        );
    }

    #[test]
    fn format_counter_cjk_decimal_negative_falls_back_to_decimal() {
        // `range: 0 infinite` override — negative is out of range, and the
        // unset `fallback` descriptor defaults to `decimal`.
        assert_eq!(format_counter(-1, &named("cjk-decimal")), "-1");
    }

    // ── §6.1 Numeric: `additive`-system styles ────────────────────────

    #[test]
    fn format_counter_armenian_reference_triple_and_alias() {
        // dfn: "(e.g., Ա, Բ, Գ, ..., ՂԸ, ՂԹ, Ճ)" for both `armenian` and
        // `upper-armenian` (`system: extends armenian` — same table).
        for name in ["armenian", "upper-armenian"] {
            assert_eq!(format_counter(1, &named(name)), "\u{531}"); // Ա
            assert_eq!(format_counter(98, &named(name)), "\u{542}\u{538}"); // ՂԸ (90+8)
            assert_eq!(format_counter(99, &named(name)), "\u{542}\u{539}"); // ՂԹ (90+9)
            assert_eq!(format_counter(100, &named(name)), "\u{543}"); // Ճ
        }
    }

    #[test]
    fn format_counter_lower_armenian_reference_triple() {
        // dfn: "(e.g., ա, բ, գ, ..., ղը, ղթ, ճ)".
        assert_eq!(format_counter(1, &named("lower-armenian")), "\u{561}"); // ա
        assert_eq!(
            format_counter(98, &named("lower-armenian")),
            "\u{572}\u{568}" // ղը
        );
        assert_eq!(format_counter(100, &named("lower-armenian")), "\u{573}"); // ճ
    }

    #[test]
    fn format_counter_armenian_range_boundaries() {
        // `range: 1 9999` shared by armenian/upper-armenian/lower-armenian.
        // 9999 = 9000 + 900 + 90 + 9 (one rep of each weight — the greedy
        // additive algorithm never repeats a weight here since Armenian's
        // table has an exact tuple for every non-zero decimal digit at
        // every place value).
        assert_eq!(
            format_counter(9999, &named("armenian")),
            "\u{554}\u{54b}\u{542}\u{539}" // ՔՋՂԹ
        );
        assert_eq!(format_counter(0, &named("armenian")), "0");
        assert_eq!(format_counter(10000, &named("armenian")), "10000");
    }

    #[test]
    fn format_counter_georgian_reference_triple_and_irregular_weight_8() {
        // dfn: "(e.g., ა, ბ, გ, ..., ჟჱ, ჟთ, რ)" — 98 uses weight-8 = ჱ
        // (U+10F1, a dedicated numeral letter reserved for 8), NOT the
        // "natural" 9th-place-in-alphabet character.
        assert_eq!(format_counter(1, &named("georgian")), "\u{10d0}"); // ა
        assert_eq!(
            format_counter(98, &named("georgian")),
            "\u{10df}\u{10f1}" // ჟჱ (90 + 8)
        );
        assert_eq!(
            format_counter(99, &named("georgian")),
            "\u{10df}\u{10d7}" // ჟთ (90 + 9)
        );
        assert_eq!(format_counter(100, &named("georgian")), "\u{10e0}"); // რ
        // 19999 = 10000 + 9000 + 900 + 90 + 9, one rep of each weight.
        assert_eq!(
            format_counter(19999, &named("georgian")),
            "\u{10f5}\u{10f0}\u{10e8}\u{10df}\u{10d7}" // ჵჰშჟთ
        );
        assert_eq!(format_counter(20000, &named("georgian")), "20000"); // range: 1 19999
    }

    #[test]
    fn format_counter_hebrew_reference_triple() {
        // dfn: "(e.g., א‎, ב‎, ג‎, ..., צח‎, צט‎, ק‎)".
        assert_eq!(format_counter(1, &named("hebrew")), "\u{5d0}"); // א
        assert_eq!(
            format_counter(98, &named("hebrew")),
            "\u{5e6}\u{5d7}" // צח (90 + 8)
        );
        assert_eq!(
            format_counter(99, &named("hebrew")),
            "\u{5e6}\u{5d8}" // צט (90 + 9)
        );
        assert_eq!(format_counter(100, &named("hebrew")), "\u{5e7}"); // ק
    }

    #[test]
    fn format_counter_hebrew_manual_override_avoids_tetragrammaton() {
        // 15/16 are manually specified as 9+6 / 9+7 rather than the
        // "natural" 10+5 / 10+6 additive decomposition, specifically to
        // avoid the two-letter combination that resembles the
        // Tetragrammaton (spec's own note on the @counter-style block).
        assert_eq!(
            format_counter(15, &named("hebrew")),
            "\u{5d8}\u{5d5}" // טו (9 + 6), not "\u{5d9}\u{5d4}" (10 + 5)
        );
        assert_eq!(
            format_counter(16, &named("hebrew")),
            "\u{5d8}\u{5d6}" // טז (9 + 7), not "\u{5d9}\u{5d5}" (10 + 6)
        );
        assert_eq!(format_counter(17, &named("hebrew")), "\u{5d9}\u{5d6}"); // יז
        assert_eq!(format_counter(19, &named("hebrew")), "\u{5d9}\u{5d8}"); // יט
    }

    #[test]
    fn format_counter_hebrew_range_boundary_falls_back_to_decimal() {
        // 10999 = 10000 + 400 + 400 + 100 + 90 + 9 (greedy: the 400-weight
        // tuple is used twice — floor(999/400) = 2, remaining 199 — then
        // 100 + 90 + 9 exactly).
        assert_eq!(
            format_counter(10999, &named("hebrew")),
            "\u{5d9}\u{5f3}\u{5ea}\u{5ea}\u{5e7}\u{5e6}\u{5d8}" // י׳תתקצט
        );
        assert_eq!(format_counter(11000, &named("hebrew")), "11000"); // range: 1 10999
        assert_eq!(format_counter(0, &named("hebrew")), "0");
    }

    #[test]
    fn format_counter_additive_named_style_is_case_insensitive() {
        assert_eq!(format_counter(1, &named("ARMENIAN")), "\u{531}");
    }

    // ── §6.2 Alphabetic: `alphabetic`-system styles beyond alpha/latin ─

    #[test]
    fn format_counter_lower_greek_wraps_at_24() {
        // dfn: "(e.g., α, β, γ, ..., ω, αα, αβ)" — 24 symbols (final sigma
        // ς deliberately absent), so ω is the 24th, not the usual
        // Greek-alphabet-adjacent 25th.
        assert_eq!(format_counter(1, &named("lower-greek")), "\u{3b1}"); // α
        assert_eq!(format_counter(24, &named("lower-greek")), "\u{3c9}"); // ω
        assert_eq!(format_counter(25, &named("lower-greek")), "\u{3b1}\u{3b1}"); // αα
        assert_eq!(format_counter(26, &named("lower-greek")), "\u{3b1}\u{3b2}"); // αβ
    }

    #[test]
    fn format_counter_hiragana_wraps_at_48() {
        // dfn: "(e.g., あ, い, う, ..., ん, ああ, あい)".
        assert_eq!(format_counter(1, &named("hiragana")), "\u{3042}"); // あ
        assert_eq!(format_counter(48, &named("hiragana")), "\u{3093}"); // ん
        assert_eq!(format_counter(49, &named("hiragana")), "\u{3042}\u{3042}"); // ああ
        assert_eq!(format_counter(50, &named("hiragana")), "\u{3042}\u{3044}"); // あい
    }

    #[test]
    fn format_counter_hiragana_iroha_wraps_at_47() {
        // dfn: "(e.g., い, ろ, は, ..., す, いい, いろ)" — 47 symbols (ん
        // excluded from iroha order), so す (not ん) is the 47th.
        assert_eq!(format_counter(1, &named("hiragana-iroha")), "\u{3044}"); // い
        assert_eq!(format_counter(47, &named("hiragana-iroha")), "\u{3059}"); // す
        assert_eq!(
            format_counter(48, &named("hiragana-iroha")),
            "\u{3044}\u{3044}" // いい
        );
        assert_eq!(
            format_counter(49, &named("hiragana-iroha")),
            "\u{3044}\u{308d}" // いろ
        );
    }

    #[test]
    fn format_counter_katakana_wraps_at_48() {
        // dfn: "(e.g., ア, イ, ウ, ..., ン, アア, アイ)".
        assert_eq!(format_counter(1, &named("katakana")), "\u{30a2}"); // ア
        assert_eq!(format_counter(48, &named("katakana")), "\u{30f3}"); // ン
        assert_eq!(format_counter(49, &named("katakana")), "\u{30a2}\u{30a2}"); // アア
    }

    #[test]
    fn format_counter_katakana_iroha_wraps_at_47() {
        // dfn: "(e.g., イ, ロ, ハ, ..., ス, イイ, イロ)".
        assert_eq!(format_counter(1, &named("katakana-iroha")), "\u{30a4}"); // イ
        assert_eq!(format_counter(47, &named("katakana-iroha")), "\u{30b9}"); // ス
        assert_eq!(
            format_counter(48, &named("katakana-iroha")),
            "\u{30a4}\u{30a4}" // イイ
        );
    }

    #[test]
    fn format_counter_alphabetic_extended_out_of_range_falls_back_to_decimal() {
        // Same strictly-positive-integer range as lower-alpha/upper-alpha.
        assert_eq!(format_counter(0, &named("lower-greek")), "0");
        assert_eq!(format_counter(-1, &named("hiragana")), "-1");
    }

    // ── §6.4 Fixed: `fixed`-system styles ─────────────────────────────

    #[test]
    fn format_counter_cjk_earthly_branch_full_range() {
        // dfn: "(e.g., 子, 丑, 寅, ..., 亥)" — 12 symbols, first value 1.
        assert_eq!(format_counter(1, &named("cjk-earthly-branch")), "\u{5b50}"); // 子
        assert_eq!(format_counter(12, &named("cjk-earthly-branch")), "\u{4ea5}"); // 亥
    }

    #[test]
    fn format_counter_cjk_heavenly_stem_full_range() {
        // dfn: "(e.g., 甲, 乙, 丙, ..., 癸)" — 10 symbols, first value 1.
        assert_eq!(format_counter(1, &named("cjk-heavenly-stem")), "\u{7532}"); // 甲
        assert_eq!(format_counter(10, &named("cjk-heavenly-stem")), "\u{7678}"); // 癸
    }

    #[test]
    fn format_counter_fixed_styles_exhausted_range_falls_back_to_decimal() {
        // Once the 12/10-symbol list is exhausted, `fixed` "cannot
        // represent" further values — decimal fallback, same as 0/negative.
        assert_eq!(format_counter(13, &named("cjk-earthly-branch")), "13");
        assert_eq!(format_counter(11, &named("cjk-heavenly-stem")), "11");
        assert_eq!(format_counter(0, &named("cjk-earthly-branch")), "0");
        assert_eq!(format_counter(-1, &named("cjk-heavenly-stem")), "-1");
    }

    // ── §7.1 Longhand East Asian: Japanese/Korean `additive` styles ───

    #[test]
    fn format_counter_japanese_informal_ten_column_reference() {
        // §7.1's 10-column reference table (0,1,2,3,10,11,99,100,101,6001).
        let style = named("japanese-informal");
        assert_eq!(format_counter(0, &style), "\u{3007}"); // 〇
        assert_eq!(format_counter(1, &style), "\u{4e00}"); // 一
        assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
        assert_eq!(format_counter(11, &style), "\u{5341}\u{4e00}"); // 十一
        assert_eq!(format_counter(99, &style), "\u{4e5d}\u{5341}\u{4e5d}"); // 九十九
        assert_eq!(format_counter(100, &style), "\u{767e}"); // 百 (no leading 一, additive table's own symbol)
        assert_eq!(format_counter(101, &style), "\u{767e}\u{4e00}"); // 百一
        assert_eq!(format_counter(6001, &style), "\u{516d}\u{5343}\u{4e00}"); // 六千一 — NO zero-digit marker
    }

    #[test]
    fn format_counter_japanese_formal_ten_column_reference() {
        let style = named("japanese-formal");
        assert_eq!(format_counter(0, &style), "\u{96f6}"); // 零
        assert_eq!(format_counter(1, &style), "\u{58f1}"); // 壱
        assert_eq!(format_counter(11, &style), "\u{58f1}\u{62fe}\u{58f1}"); // 壱拾壱
        assert_eq!(format_counter(100, &style), "\u{58f1}\u{767e}"); // 壱百
        assert_eq!(
            format_counter(6001, &style),
            "\u{516d}\u{9621}\u{58f1}" // 六阡壱
        );
    }

    #[test]
    fn format_counter_korean_hangul_formal_ten_column_reference() {
        let style = named("korean-hangul-formal");
        assert_eq!(format_counter(0, &style), "\u{c601}"); // 영
        assert_eq!(format_counter(1, &style), "\u{c77c}"); // 일
        assert_eq!(
            format_counter(11, &style),
            "\u{c77c}\u{c2ed}\u{c77c}" // 일십일
        );
        assert_eq!(format_counter(100, &style), "\u{c77c}\u{bc31}"); // 일백
        assert_eq!(
            format_counter(6001, &style),
            "\u{c721}\u{cc9c}\u{c77c}" // 육천일
        );
    }

    #[test]
    fn format_counter_korean_hanja_informal_and_formal_ten_column_reference() {
        let informal = named("korean-hanja-informal");
        let formal = named("korean-hanja-formal");
        assert_eq!(format_counter(11, &informal), "\u{5341}\u{4e00}"); // 十一
        assert_eq!(format_counter(6001, &informal), "\u{516d}\u{5343}\u{4e00}"); // 六千一
        assert_eq!(
            format_counter(11, &formal),
            "\u{58f9}\u{62fe}\u{58f9}" // 壹拾壹
        );
        assert_eq!(
            format_counter(6001, &formal),
            "\u{516d}\u{4edf}\u{58f9}" // 六仟壹
        );
    }

    #[test]
    fn format_counter_longhand_additive_negative_uses_style_specific_prefix() {
        assert_eq!(
            format_counter(-1, &named("japanese-informal")),
            "\u{30de}\u{30a4}\u{30ca}\u{30b9}\u{4e00}" // マイナス一
        );
        assert_eq!(
            format_counter(-11, &named("korean-hangul-formal")),
            "\u{b9c8}\u{c774}\u{b108}\u{c2a4}  \u{c77c}\u{c2ed}\u{c77c}" // 마이너스  일십일 (two spaces)
        );
    }

    #[test]
    fn format_counter_longhand_additive_out_of_range_falls_back_through_cjk_decimal() {
        // Two-hop chain: out of `-9999..=9999` → cjk-decimal (positional);
        // cjk-decimal's own `range: 0 infinite` still covers this positive
        // out-of-range value, so it does NOT fall through to plain decimal
        // here — 10000 renders as cjk-decimal's "一〇〇〇〇", not "10000".
        assert_eq!(
            format_counter(10000, &named("japanese-informal")),
            "\u{4e00}\u{3007}\u{3007}\u{3007}\u{3007}" // 一〇〇〇〇 (cjk-decimal)
        );
        // Negative out of range: cjk-decimal also rejects it (range starts
        // at 0), so the chain falls all the way through to plain decimal.
        assert_eq!(
            format_counter(-10000, &named("japanese-informal")),
            "-10000"
        );
    }

    // ── §7.1.3 Longhand East Asian: Chinese digit-marker styles ───────

    #[test]
    fn format_counter_simp_chinese_informal_ten_column_reference() {
        let style = named("simp-chinese-informal");
        assert_eq!(format_counter(0, &style), "\u{96f6}"); // 零
        assert_eq!(format_counter(1, &style), "\u{4e00}"); // 一
        assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
        assert_eq!(format_counter(11, &style), "\u{5341}\u{4e00}"); // 十一
        assert_eq!(format_counter(99, &style), "\u{4e5d}\u{5341}\u{4e5d}"); // 九十九
        assert_eq!(format_counter(100, &style), "\u{4e00}\u{767e}"); // 一百 (leading 一, unlike japanese-informal)
        assert_eq!(
            format_counter(101, &style),
            "\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 一百零一
        );
        assert_eq!(
            format_counter(6001, &style),
            "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一 — HAS the zero-digit marker
        );
    }

    #[test]
    fn format_counter_simp_chinese_informal_first_120_values_spot_check() {
        // Spot-checks against the spec's own first-120-values worked
        // table, hitting the 10-19 tens-digit-removal boundary and the
        // point where 110/111 do NOT get that same removal.
        let style = named("simp-chinese-informal");
        assert_eq!(format_counter(9, &style), "\u{4e5d}"); // 九
        assert_eq!(format_counter(10, &style), "\u{5341}"); // 十
        assert_eq!(format_counter(19, &style), "\u{5341}\u{4e5d}"); // 十九
        assert_eq!(format_counter(20, &style), "\u{4e8c}\u{5341}"); // 二十
        assert_eq!(format_counter(21, &style), "\u{4e8c}\u{5341}\u{4e00}"); // 二十一
        assert_eq!(
            format_counter(109, &style),
            "\u{4e00}\u{767e}\u{96f6}\u{4e5d}"
        ); // 一百零九
        assert_eq!(
            format_counter(110, &style),
            "\u{4e00}\u{767e}\u{4e00}\u{5341}"
        ); // 一百一十 (no removal at 110)
        assert_eq!(
            format_counter(111, &style),
            "\u{4e00}\u{767e}\u{4e00}\u{5341}\u{4e00}" // 一百一十一
        );
        assert_eq!(
            format_counter(120, &style),
            "\u{4e00}\u{767e}\u{4e8c}\u{5341}"
        ); // 一百二十
    }

    #[test]
    fn format_counter_simp_chinese_formal_does_not_drop_tens_digit() {
        // Formal styles skip algorithm step 3 (informal-only tens-digit
        // removal) entirely — 10/11 keep their leading 壹.
        let style = named("simp-chinese-formal");
        assert_eq!(format_counter(10, &style), "\u{58f9}\u{62fe}"); // 壹拾
        assert_eq!(format_counter(11, &style), "\u{58f9}\u{62fe}\u{58f9}"); // 壹拾壹
        assert_eq!(format_counter(100, &style), "\u{58f9}\u{4f70}"); // 壹佰
        assert_eq!(
            format_counter(101, &style),
            "\u{58f9}\u{4f70}\u{96f6}\u{58f9}" // 壹佰零壹
        );
        assert_eq!(
            format_counter(6001, &style),
            "\u{9646}\u{4edf}\u{96f6}\u{58f9}" // 陆仟零壹 (simp 6 = 陆)
        );
    }

    #[test]
    fn format_counter_trad_chinese_formal_uses_traditional_glyphs() {
        // Differs from simp-chinese-formal only in digits 2/3/6 and the
        // negative sign — verified via digit 6 (simp 陆 U+9646 vs trad 陸
        // U+9678) and the negative sign (简负 U+8D1F vs 繁負 U+8CA0).
        let style = named("trad-chinese-formal");
        assert_eq!(
            format_counter(6001, &style),
            "\u{9678}\u{4edf}\u{96f6}\u{58f9}" // 陸仟零壹
        );
        assert_eq!(
            format_counter(-1, &style),
            "\u{8ca0}\u{58f9}" // 負壹
        );
    }

    #[test]
    fn format_counter_trad_chinese_informal_matches_simp_except_negative_sign() {
        // `trad-chinese-informal`'s digit/marker table is byte-identical to
        // `simp-chinese-informal`'s (informal-style basic numerals don't
        // differ between simplified/traditional) — the only distinguishing
        // field is the negative sign (負 U+8CA0 vs 负 U+8D1F). A
        // positive-value assertion alone couldn't tell this entry apart
        // from an accidental `simp-chinese-informal` reuse, so the
        // negative case is load-bearing here.
        let style = named("trad-chinese-informal");
        assert_eq!(
            format_counter(6001, &style),
            "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一
        );
        assert_eq!(
            format_counter(-101, &style),
            "\u{8ca0}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 負一百零一
        );
    }

    #[test]
    fn format_counter_cjk_ideographic_matches_trad_chinese_informal() {
        // §7.1.3's own dfn: "cjk-ideographic ... identical to
        // trad-chinese-informal. (It exists for legacy reasons.)" — pin
        // both a positive and the negative case (same rationale as the
        // trad-chinese-informal-vs-simp-chinese-informal test above: a
        // positive-only assertion can't distinguish "correctly aliased to
        // trad-chinese-informal" from "accidentally aliased to
        // simp-chinese-informal", since the two tables only diverge on the
        // negative-sign glyph).
        let style = named("cjk-ideographic");
        assert_eq!(
            format_counter(6001, &style),
            "\u{516d}\u{5343}\u{96f6}\u{4e00}" // 六千零一
        );
        assert_eq!(
            format_counter(-101, &style),
            "\u{8ca0}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 負一百零一
        );
    }

    #[test]
    fn format_counter_chinese_digit_marker_negative_uses_single_char_prefix() {
        assert_eq!(
            format_counter(-101, &named("simp-chinese-informal")),
            "\u{8d1f}\u{4e00}\u{767e}\u{96f6}\u{4e00}" // 负一百零一
        );
    }

    #[test]
    fn format_counter_chinese_digit_marker_out_of_range_falls_back_through_cjk_decimal() {
        assert_eq!(
            format_counter(10000, &named("simp-chinese-informal")),
            "\u{4e00}\u{3007}\u{3007}\u{3007}\u{3007}" // cjk-decimal 一〇〇〇〇
        );
        assert_eq!(
            format_counter(-10000, &named("trad-chinese-formal")),
            "-10000" // cjk-decimal also rejects negative, falls to plain decimal
        );
    }

    // ── §7.2 Ethiopic Numeric Counter Style ───────────────────────────

    #[test]
    fn format_counter_ethiopic_numeric_value_one_special_case() {
        assert_eq!(format_counter(1, &named("ethiopic-numeric")), "\u{1369}"); // ፩
    }

    #[test]
    fn format_counter_ethiopic_numeric_spec_worked_examples() {
        // The spec's own first two worked examples fit `i32` directly.
        assert_eq!(format_counter(100, &named("ethiopic-numeric")), "\u{137b}"); // ፻
        assert_eq!(
            format_counter(78_010_092, &named("ethiopic-numeric")),
            "\u{1378}\u{1370}\u{137b}\u{1369}\u{137c}\u{137a}\u{136a}" // ፸፰፻፩፼፺፪
        );
        // The spec's third example (780100000092 → ...፼፼...) doesn't fit
        // `i32` (counter values are `i32` throughout this file), so it
        // can't be exercised through this API. 10000092 substitutes: same
        // asymmetry (an even, non-zero index group with value 0 still
        // emits a bare ፼ with no preceding digit — group 2 here), verified
        // in-range by hand-tracing the algorithm: groups (LSB-first)
        // `[92, 0, 0, 10]`; group 3 (msb, odd, value 10) → ፲፻; group 2
        // (even, index != 0, value 0) → suppressed digit, but it still
        // gets a bare ፼; group 1 (odd, value 0) → fully suppressed
        // (exception applies to ፻ only); group 0 → ፺፪.
        assert_eq!(
            format_counter(10_000_092, &named("ethiopic-numeric")),
            "\u{1372}\u{137b}\u{137c}\u{137a}\u{136a}" // ፲፻፼፺፪
        );
    }

    #[test]
    fn format_counter_ethiopic_numeric_out_of_range_falls_back_to_decimal() {
        // `range: 1 infinite` — 0 and negative are out of range.
        assert_eq!(format_counter(0, &named("ethiopic-numeric")), "0");
        assert_eq!(format_counter(-1, &named("ethiopic-numeric")), "-1");
    }

    #[test]
    fn join_counter_stack_decimal_with_dot_separator() {
        assert_eq!(
            join_counter_stack(&[1, 2, 3], ".", &CounterStyle::Decimal),
            "1.2.3"
        );
        assert_eq!(join_counter_stack(&[5], ".", &CounterStyle::Decimal), "5");
        assert_eq!(join_counter_stack(&[], ".", &CounterStyle::Decimal), "");
    }

    #[test]
    fn join_counter_stack_non_decimal_style() {
        // target-counters() path (TargetRequest::Counters) threads `style`
        // through join_counter_stack the same as the single-value path —
        // pin that a non-decimal style formats every level, not just the
        // leaf.
        let style = CounterStyle::Named(SmolStr::new("lower-alpha"));
        assert_eq!(join_counter_stack(&[1, 2, 3], ".", &style), "a.b.c");
    }

    // ── target-counter resolve (immediate path) ─────────────────────

    fn make_info(counters: &[(&str, &[i32])], texts: &[(ContentPart, &str)]) -> TargetInfo {
        let mut info = TargetInfo::default();
        for (name, stack) in counters {
            info.counters.insert(Symbol::new(*name), stack.to_vec());
        }
        for (part, text) in texts {
            info.text_parts.push((*part, (*text).to_owned()));
        }
        info
    }

    #[test]
    fn resolve_target_counter_returns_leaf_of_stack() {
        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("chapter-3"),
            make_info(&[("chapter", &[1, 2, 3])], &[]),
        );
        let out =
            reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
        assert_eq!(out, ResolveOutcome::Resolved("3".to_owned()));
        // Immediate resolve must not queue a pending slot.
        assert!(reg.pending_slots.is_empty());
    }

    #[test]
    fn resolve_target_counter_missing_counter_defaults_to_zero() {
        let mut reg = TargetRegistry::default();
        reg.register(Symbol::new("chapter-3"), make_info(&[], &[]));
        let out =
            reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
        assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
    }

    #[test]
    fn resolve_target_counter_non_fragment_url_falls_back_to_empty() {
        let mut reg = TargetRegistry::default();
        let out = reg.resolve_target_counter(
            "https://example.com/",
            Symbol::new("chapter"),
            CounterStyle::Decimal,
        );
        assert_eq!(out, ResolveOutcome::Resolved(String::new()));
        // Fallback must not queue a pending slot (would leak into flush).
        assert!(reg.pending_slots.is_empty());
    }

    // ── target-counters resolve (immediate path) ────────────────────

    #[test]
    fn resolve_target_counters_joins_full_stack() {
        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("sec-1-2-3"),
            make_info(&[("section", &[1, 2, 3])], &[]),
        );
        let out = reg.resolve_target_counters(
            "#sec-1-2-3",
            Symbol::new("section"),
            ".",
            CounterStyle::Decimal,
        );
        assert_eq!(out, ResolveOutcome::Resolved("1.2.3".to_owned()));
    }

    #[test]
    fn resolve_target_counters_absent_counter_yields_zero() {
        // CSS Content 3 §2.6.2: an undefined counter has value 0;
        // `counters(name, sep)` renders that as the single formatted "0"
        // (join-with-sep of a one-element `[0]`) — NOT the empty string.
        // Regression pin against a prior implementation that used
        // `.unwrap_or_default()` on the joined output.
        let mut reg = TargetRegistry::default();
        reg.register(Symbol::new("sec-1"), make_info(&[], &[]));
        let out = reg.resolve_target_counters(
            "#sec-1",
            Symbol::new("section"),
            ".",
            CounterStyle::Decimal,
        );
        assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
    }

    #[test]
    fn resolve_target_counters_empty_stack_yields_zero() {
        // Distinct code path from "absent counter": the counter *is* present
        // in `counters` but its stack is an empty Vec. Same CSS §2.6.2 rule
        // applies — render as "0".
        let mut reg = TargetRegistry::default();
        reg.register(Symbol::new("sec-1"), make_info(&[("section", &[])], &[]));
        let out = reg.resolve_target_counters(
            "#sec-1",
            Symbol::new("section"),
            ".",
            CounterStyle::Decimal,
        );
        assert_eq!(out, ResolveOutcome::Resolved("0".to_owned()));
    }

    #[test]
    fn resolve_target_counters_non_fragment_url_falls_back_to_empty() {
        // Analogous to resolve_target_counter_non_fragment_url_falls_back_to_empty:
        // non-fragment URLs (external, malformed) are unresolvable
        // per-document and must NOT queue a pending slot (would grow
        // pending_slots unbounded).
        let mut reg = TargetRegistry::default();
        let out = reg.resolve_target_counters(
            "https://example.com/",
            Symbol::new("section"),
            ".",
            CounterStyle::Decimal,
        );
        assert_eq!(out, ResolveOutcome::Resolved(String::new()));
        assert!(reg.pending_slots.is_empty());
    }

    // ── target-text resolve (immediate path) ────────────────────────

    #[test]
    fn resolve_target_text_returns_content_part() {
        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("h1"),
            make_info(
                &[],
                &[
                    (ContentPart::Content, "Introduction"),
                    (ContentPart::Before, "§"),
                ],
            ),
        );
        assert_eq!(
            reg.resolve_target_text("#h1", ContentPart::Content),
            ResolveOutcome::Resolved("Introduction".to_owned())
        );
        assert_eq!(
            reg.resolve_target_text("#h1", ContentPart::Before),
            ResolveOutcome::Resolved("§".to_owned())
        );
    }

    #[test]
    fn resolve_target_text_absent_part_yields_empty_string() {
        let mut reg = TargetRegistry::default();
        reg.register(Symbol::new("h1"), make_info(&[], &[]));
        assert_eq!(
            reg.resolve_target_text("#h1", ContentPart::Content),
            ResolveOutcome::Resolved(String::new())
        );
    }

    #[test]
    fn resolve_target_text_non_fragment_url_falls_back_to_empty() {
        // Analogous to the counter/counters non-fragment tests: unresolvable
        // external URLs must return an empty string synchronously without
        // queueing a pending slot.
        let mut reg = TargetRegistry::default();
        let out = reg.resolve_target_text("https://example.com/", ContentPart::Content);
        assert_eq!(out, ResolveOutcome::Resolved(String::new()));
        assert!(reg.pending_slots.is_empty());
    }

    // ── Pending / flush_pending (forward-reference path) ────────────

    #[test]
    fn forward_reference_yields_pending_then_flushes_to_resolved() {
        let mut reg = TargetRegistry::default();

        // Site A resolves *before* the register site has been walked.
        let out_a =
            reg.resolve_target_counter("#chapter-3", Symbol::new("chapter"), CounterStyle::Decimal);
        let id_a = match out_a {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
        };
        assert_eq!(
            id_a,
            TargetSlotId {
                page_index: 0,
                sequence: 0,
            }
        );
        assert_eq!(reg.pending_slots.len(), 1);

        // Second forward reference to a different fragment / kind.
        let out_b = reg.resolve_target_text("#chapter-3", ContentPart::Content);
        let id_b = match out_b {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
        };
        assert_eq!(
            id_b,
            TargetSlotId {
                page_index: 0,
                sequence: 1,
            }
        );
        assert_eq!(reg.pending_slots.len(), 2);

        // Walk lands the register.
        reg.register(
            Symbol::new("chapter-3"),
            make_info(
                &[("chapter", &[1, 2, 3])],
                &[(ContentPart::Content, "The Middle")],
            ),
        );

        // Flush drains both pending slots and returns their sequenced values.
        let resolutions = reg.flush_pending();
        assert_eq!(resolutions.len(), 2);
        assert!(reg.pending_slots.is_empty());
        assert_eq!(
            resolutions[0],
            PendingResolution {
                slot_id: id_a,
                value: Some("3".to_owned()),
            }
        );
        assert_eq!(
            resolutions[1],
            PendingResolution {
                slot_id: id_b,
                value: Some("The Middle".to_owned()),
            }
        );
    }

    #[test]
    fn flush_pending_retains_unregistered_slots() {
        // Streaming intent (design §7.4): a slot whose fragment never landed
        // must NOT be dropped — it may resolve in a later register + flush
        // cycle (e.g. next batch, next Consumer iteration).
        let mut reg = TargetRegistry::default();
        let out =
            reg.resolve_target_counter("#never", Symbol::new("chapter"), CounterStyle::Decimal);
        assert!(matches!(out, ResolveOutcome::Pending(_)));
        assert_eq!(reg.pending_slots.len(), 1);

        // First flush: no register, slot retained.
        let resolutions = reg.flush_pending();
        assert!(resolutions.is_empty());
        assert_eq!(reg.pending_slots.len(), 1);

        // Later register lands the fragment; second flush resolves it.
        reg.register(Symbol::new("never"), make_info(&[("chapter", &[7])], &[]));
        let resolutions = reg.flush_pending();
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].value.as_deref(), Some("7"));
        assert!(reg.pending_slots.is_empty());
    }

    #[test]
    fn register_first_wins_on_duplicate_fragment_id() {
        // DOM id resolution is first-in-tree-order (HTML §3.2.6.1 —
        // duplicate ids are invalid HTML but `getElementById` still resolves
        // to the first element in tree order). register() is expected to be
        // called in tree order by the register-site walker; last-wins would
        // cause target-* to resolve against a later duplicate.
        let mut reg = TargetRegistry::default();
        reg.register(Symbol::new("dup"), make_info(&[("chapter", &[1])], &[]));
        // Second register call for the same fragment must be a no-op.
        reg.register(
            Symbol::new("dup"),
            make_info(&[("chapter", &[99])], &[(ContentPart::Content, "later")]),
        );

        // Counter resolves against the first (winning) register.
        let out = reg.resolve_target_counter("#dup", Symbol::new("chapter"), CounterStyle::Decimal);
        assert_eq!(out, ResolveOutcome::Resolved("1".to_owned()));

        // Text side: the first info had no ContentPart::Content, so the
        // fallback is "" — NOT "later" from the second (losing) info.
        // This pins that or_insert isn't quietly merging fields.
        let out_text = reg.resolve_target_text("#dup", ContentPart::Content);
        assert_eq!(out_text, ResolveOutcome::Resolved(String::new()));
    }

    #[test]
    fn flush_pending_partial_preserves_order_and_retains_unresolved() {
        // Mixed batch: some slots resolve, some don't. The flush must
        //   (a) emit resolutions in slot-queued order (sequence 0 before 1
        //       before 3, etc.),
        //   (b) retain unresolved slots in `pending_slots` for a later
        //       flush cycle,
        //   (c) not disturb the retained slots' original ordering.
        let mut reg = TargetRegistry::default();

        // Queue three slots for different fragments in this order:
        //   seq 0: #a  counter
        //   seq 1: #b  counters
        //   seq 2: #c  text
        //   seq 3: #a  text  (same fragment as seq 0)
        let id_a =
            match reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal) {
                ResolveOutcome::Pending(s) => s,
                ResolveOutcome::Resolved(_) => panic!("expected Pending"),
            };
        let id_b = match reg.resolve_target_counters(
            "#b",
            Symbol::new("section"),
            "-",
            CounterStyle::Decimal,
        ) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        let id_c = match reg.resolve_target_text("#c", ContentPart::Content) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        let id_a2 = match reg.resolve_target_text("#a", ContentPart::Before) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        assert_eq!(
            (id_a.sequence, id_b.sequence, id_c.sequence, id_a2.sequence),
            (0, 1, 2, 3)
        );
        assert!(
            [id_a, id_b, id_c, id_a2]
                .iter()
                .all(|id| id.page_index == 0),
            "no page boundary crossed in this test — every slot stays on page 0"
        );
        assert_eq!(reg.pending_slots.len(), 4);

        // Register #a only. #b and #c are still absent.
        reg.register(
            Symbol::new("a"),
            make_info(&[("chapter", &[7])], &[(ContentPart::Before, "prefix")]),
        );

        let resolutions = reg.flush_pending();

        // Two slots resolved (both for #a), two retained (for #b and #c).
        assert_eq!(resolutions.len(), 2);
        assert_eq!(reg.pending_slots.len(), 2);

        // Order preservation: resolutions came out in original queue order —
        // seq 0 (#a counter) before seq 3 (#a text).
        assert_eq!(
            resolutions[0],
            PendingResolution {
                slot_id: id_a,
                value: Some("7".to_owned()),
            }
        );
        assert_eq!(
            resolutions[1],
            PendingResolution {
                slot_id: id_a2,
                value: Some("prefix".to_owned()),
            }
        );

        // Retained slots keep their original ordering (seq 1 before seq 2).
        assert_eq!(reg.pending_slots[0].id, id_b);
        assert_eq!(reg.pending_slots[1].id, id_c);

        // Registering #b later resolves it on the next flush; #c stays.
        reg.register(Symbol::new("b"), make_info(&[("section", &[1, 2])], &[]));
        let resolutions2 = reg.flush_pending();
        assert_eq!(resolutions2.len(), 1);
        assert_eq!(resolutions2[0].slot_id, id_b);
        assert_eq!(resolutions2[0].value.as_deref(), Some("1-2"));
        assert_eq!(reg.pending_slots.len(), 1);
        assert_eq!(reg.pending_slots[0].id, id_c);
    }

    #[test]
    fn flush_pending_counters_variant_joins_stack() {
        // Pending path for target-counters must remember `separator` and
        // `style`, not just the counter `name` — regression pin for the
        // TargetRequest::Counters variant fields.
        let mut reg = TargetRegistry::default();
        let out = reg.resolve_target_counters(
            "#sec-1-2-3",
            Symbol::new("section"),
            "-",
            CounterStyle::Decimal,
        );
        assert!(matches!(out, ResolveOutcome::Pending(_)));

        reg.register(
            Symbol::new("sec-1-2-3"),
            make_info(&[("section", &[1, 2, 3])], &[]),
        );
        let resolutions = reg.flush_pending();
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].value.as_deref(), Some("1-2-3"));
    }

    // ── TargetSlotId pairing / begin_page ────

    #[test]
    fn begin_page_resets_sequence_and_advances_page_index() {
        // Design §7.6 "Slot ID の安定性保証": sequence is page-local
        // (0-indexed within the page), so a page-boundary call must restart
        // the local count. The same local sequence value on two different
        // pages must therefore produce two distinct TargetSlotIds.
        let mut reg = TargetRegistry::default();

        let out_page0 =
            reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal);
        let id_page0 = match out_page0 {
            ResolveOutcome::Pending(id) => id,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        assert_eq!(
            id_page0,
            TargetSlotId {
                page_index: 0,
                sequence: 0,
            }
        );

        reg.begin_page(1);
        let out_page1 =
            reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);
        let id_page1 = match out_page1 {
            ResolveOutcome::Pending(id) => id,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };

        assert_eq!(
            id_page1,
            TargetSlotId {
                page_index: 1,
                sequence: 0,
            },
            "begin_page must reset the local sequence counter to 0"
        );
        assert_eq!(
            id_page0.sequence, id_page1.sequence,
            "both slots are each page's first dispatch — same local sequence"
        );
        assert_ne!(
            id_page0, id_page1,
            "same local sequence on different pages must yield distinct TargetSlotIds"
        );
    }

    #[test]
    fn begin_page_same_index_is_a_no_op() {
        // A redundant begin_page call for the page already in progress must
        // not reset the count out from under slots already queued this
        // page — only an actual page transition resets `next_sequence`.
        let mut reg = TargetRegistry::default();
        reg.begin_page(0); // already page 0 — no-op
        let out_a = reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal);
        reg.begin_page(0); // redundant same-page call — still a no-op
        let out_b = reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);

        let (id_a, id_b) = match (out_a, out_b) {
            (ResolveOutcome::Pending(a), ResolveOutcome::Pending(b)) => (a, b),
            _ => panic!("expected both Pending"),
        };
        assert_eq!(id_a.page_index, 0);
        assert_eq!(id_b.page_index, 0);
        assert_eq!(
            (id_a.sequence, id_b.sequence),
            (0, 1),
            "redundant same-index begin_page must not reset the in-progress page's sequence"
        );
    }

    #[test]
    #[should_panic(expected = "page_index must be monotonically non-decreasing")]
    fn begin_page_backward_call_panics_instead_of_duplicating_slot_id() {
        // Without a monotonicity guard, a backward begin_page() call resets
        // next_sequence to 0 while an earlier page's slot is still
        // unresolved in pending_slots (unresolved slots are retained across
        // flushes forever — see flush_pending_retains_unregistered_slots).
        // The very next dispatch on the revisited page would then mint a
        // TargetSlotId byte-identical to the still-pending one, breaking
        // the uniqueness guarantee TargetSlotId exists to provide
        // (crate::error::TargetSlotId's doc comment). This must now panic
        // instead of silently corrupting pending_slots.
        let mut reg = TargetRegistry::default();

        // Step 1: begin_page(0), dispatch an unresolved target — mints
        // TargetSlotId{page_index:0, sequence:0}, retained in pending_slots
        // (its fragment "#never-registered" is never registered).
        let out_page0 = reg.resolve_target_counter(
            "#never-registered",
            Symbol::new("chapter"),
            CounterStyle::Decimal,
        );
        let id_page0 = match out_page0 {
            ResolveOutcome::Pending(id) => id,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        assert_eq!(
            id_page0,
            TargetSlotId {
                page_index: 0,
                sequence: 0,
            }
        );
        assert_eq!(reg.pending_slots.len(), 1, "slot from page 0 still queued");

        // Step 2: begin_page(1), more dispatches on page 1.
        reg.begin_page(1);
        let _ = reg.resolve_target_counter("#b", Symbol::new("chapter"), CounterStyle::Decimal);

        // Step 3: begin_page(0) again — a backward call (convergence
        // re-run / two-pass layout walk / driver bug). This must panic
        // rather than reset next_sequence out from under the still-pending
        // page-0 slot from step 1.
        reg.begin_page(0);
    }

    #[test]
    fn target_slot_id_is_stable_across_repeated_construction() {
        // Byte-identical goal (design §7.4 / §7.6, Finding #4): constructing
        // the same (page_index, sequence) pair twice must compare equal and
        // hash identically — Consumer patch tables key off this.
        let a = TargetSlotId {
            page_index: 2,
            sequence: 5,
        };
        let b = TargetSlotId {
            page_index: 2,
            sequence: 5,
        };
        assert_eq!(a, b);

        let mut set = std::collections::HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    // ── ContentComponent wire-through (conversion path) ─────

    #[test]
    fn resolve_content_component_drives_target_counter() {
        let cc = ContentComponent::TargetCounter {
            url: "#chapter-3".to_owned(),
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        };

        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("chapter-3"),
            make_info(&[("chapter", &[1, 2, 3])], &[]),
        );

        let outcome = resolve_content_component(&mut reg, &cc)
            .expect("target-* variant should return Some(outcome)");
        assert_eq!(outcome, ResolveOutcome::Resolved("3".to_owned()));
    }

    #[test]
    fn resolve_content_component_drives_target_counters() {
        let cc = ContentComponent::TargetCounters {
            url: "#sec-1-2".to_owned(),
            name: SmolStr::new("section"),
            separator: ".".to_owned(),
            style: CounterStyle::Decimal,
        };

        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("sec-1-2"),
            make_info(&[("section", &[1, 2])], &[]),
        );

        let outcome = resolve_content_component(&mut reg, &cc)
            .expect("target-* variant should return Some(outcome)");
        assert_eq!(outcome, ResolveOutcome::Resolved("1.2".to_owned()));
    }

    #[test]
    fn resolve_content_component_drives_target_text() {
        let cc = ContentComponent::TargetText {
            url: "#h1".to_owned(),
            part: ContentPart::Content,
        };

        let mut reg = TargetRegistry::default();
        reg.register(
            Symbol::new("h1"),
            make_info(&[], &[(ContentPart::Content, "Hello")]),
        );

        let outcome = resolve_content_component(&mut reg, &cc)
            .expect("target-* variant should return Some(outcome)");
        assert_eq!(outcome, ResolveOutcome::Resolved("Hello".to_owned()));
    }

    #[test]
    fn resolve_content_component_non_fragment_url_falls_back_to_empty() {
        // Every target-* variant driven through resolve_content_component
        // must inherit the non-fragment fallback (unresolvable per-document,
        // no pending slot queued).
        let mut reg = TargetRegistry::default();

        let counter_cc = ContentComponent::TargetCounter {
            url: "https://example.com/".to_owned(),
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        };
        assert_eq!(
            resolve_content_component(&mut reg, &counter_cc),
            Some(ResolveOutcome::Resolved(String::new()))
        );

        let counters_cc = ContentComponent::TargetCounters {
            url: "http://elsewhere/".to_owned(),
            name: SmolStr::new("section"),
            separator: ".".to_owned(),
            style: CounterStyle::Decimal,
        };
        assert_eq!(
            resolve_content_component(&mut reg, &counters_cc),
            Some(ResolveOutcome::Resolved(String::new()))
        );

        let text_cc = ContentComponent::TargetText {
            url: "".to_owned(),
            part: ContentPart::Content,
        };
        assert_eq!(
            resolve_content_component(&mut reg, &text_cc),
            Some(ResolveOutcome::Resolved(String::new()))
        );

        // All three fallbacks must have been synchronous — no pending slots
        // must have been queued for the unresolvable URLs.
        assert!(reg.pending_slots.is_empty());
    }

    #[test]
    fn resolve_content_component_returns_none_for_non_target_variants() {
        // Every non-target ContentComponent variant must decline the resolve
        // — the pending queue must not grow for Literal / Counter / etc.
        // Guards against future non_exhaustive variants: catch-all in
        // resolve_content_component keeps them None-by-default (safe:
        // resolve pass ignores them, other passes will handle them).
        let mut reg = TargetRegistry::default();

        let literal = ContentComponent::Literal(SmolStr::new("hello"));
        assert!(resolve_content_component(&mut reg, &literal).is_none());

        let counter = ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        };
        assert!(resolve_content_component(&mut reg, &counter).is_none());

        let attr = ContentComponent::Attr {
            name: SmolStr::new("href"),
        };
        assert!(resolve_content_component(&mut reg, &attr).is_none());

        // No non-target dispatch should have queued a pending slot.
        assert!(reg.pending_slots.is_empty());
    }
}
