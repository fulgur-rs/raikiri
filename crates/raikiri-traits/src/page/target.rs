//! GCPM `target-*()` / `element()` runtime resolve — canonical registry.
//!
//! **Ownership** (design §7.0 line 1904 "shared types → raikiri-traits"): this
//! is the single canonical [`TargetRegistry`] shared across raikiri-dom,
//! raikiri-paint, and consumer crates. raikiri-traits owns the type; the
//! producer side (register-site directive walker feeding
//! [`raikiri_traits::GcpmDirective::RegisterTarget`]) lives in raikiri-dom and
//! calls [`TargetRegistry::register`] against this canonical instance.
//!
//! **History** — the impl and tests here were previously a `pub(crate)` shadow
//! at `crates/raikiri-dom/src/target.rs` alongside an empty
//! `#[non_exhaustive]` placeholder at `crates/raikiri-traits/src/page.rs`.
//! bd raikiri-spike-bsi (Sprint 15 dom-3 Wave 1, Option C) merged the two:
//! the dom-side shadow was deleted, the field layout / method surface was
//! promoted from `pub(crate)` to `pub`, and this file now hosts the canonical
//! type.
//!
//! **Canonical shape** (design §7.2 lines 1961-1964):
//! ```text
//! pub struct TargetRegistry {
//!     resolved: HashMap<Symbol, TargetInfo>,
//!     pending_slots: Vec<TargetSlot>,
//! }
//! ```
//! `next_sequence: u32` is an additive internal-only field stamping stable
//! ids on pending slots; §7.2 does not enumerate it because
//! `TargetSlotId = (page_index, sequence)` (§11.2 Finding #4) fixes the paired
//! shape once PageContext / page_index plumbing lands. Until then the
//! sequence alone is the stable handle.

use std::collections::HashMap;

use raikiri_style::property::{ContentComponent, ContentPart, CounterStyle};

use crate::dom::Symbol;

/// Runtime registry for GCPM `target-*()` / `element()` fragment references.
///
/// A single `TargetRegistry` per document collects (a) fragments that have
/// been walked and can answer `target-counter` / `target-counters` /
/// `target-text` queries directly and (b) pending content-value sites that
/// referenced a fragment before it was walked and must be resolved on a
/// second pass ([`TargetRegistry::flush_pending`]).
///
/// Producer / register-site walker (bd raikiri-spike-96u.4) lives in
/// raikiri-dom; the canonical type (shape + resolve strategy) lives here
/// (bd raikiri-spike-bsi Option C).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetRegistry {
    /// Fragment identifier → resolved target metadata (counter snapshot,
    /// textual content parts). Populated as the runtime walk encounters
    /// [`crate::GcpmDirective::RegisterTarget`] directives (populator: bd
    /// raikiri-spike-96u.4 register-site walker in raikiri-dom).
    resolved: HashMap<Symbol, TargetInfo>,
    /// Content-value sites (`target-counter(...)`, `target-counters(...)`,
    /// `target-text(...)`, and future `element(name)`) that referenced a
    /// fragment before it was resolved. Flushed on a second pass via
    /// [`TargetRegistry::flush_pending`] once the registry is fully
    /// populated. Slots for fragments that never get registered remain in
    /// the queue after a flush (streaming intent: a later batch or
    /// finish_render pass may still land them; see design §7.4).
    pending_slots: Vec<TargetSlot>,
    /// Monotonic sequence stamp for pending slots. Not in the §7.2 shape —
    /// see module-level "canonical shape" note.
    next_sequence: u32,
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
/// reconciliation must widen `counter_snapshot` to a stack — and the 96u.4
/// register-site directive walker must populate the stack rather than the
/// leaf value.
///
/// **Text parts**: keyed by the same
/// [`raikiri_style::property::ContentPart`] variants that
/// `target-text(url, part)` accepts. Missing parts resolve to an empty
/// string (CSS Content 3 §2.6.3 defers text extraction — populating each
/// entry is the responsibility of the register site, bd raikiri-spike-96u.4).
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
/// [`ResolveOutcome::Pending`]'s sequence handle and later
/// [`PendingResolution`].
///
/// **Sequence vs. design's `TargetSlotId = (page_index, sequence)`**
/// (§11.2 Finding #4): the design pairs the sequence with `page_index` so
/// (a) slot ids stay byte-identical across iterations and (b) sinks can
/// address a slot by its owning page. `page_index` is a PageContext-owned
/// field and is out of current scope. Until the PageContext plumbing lands
/// (bd raikiri-spike-96u.4), the sequence alone is the stable handle.
#[derive(Debug, Clone)]
pub(crate) struct TargetSlot {
    pub(crate) sequence: u32,
    pub(crate) fragment_id: Symbol,
    pub(crate) request: TargetRequest,
}

/// Outcome of a `resolve_target_*` call.
///
/// `Resolved(String)` — the fragment was in `resolved` (or the URL was a
/// non-fragment fallback), the formatted answer is returned inline.
///
/// `Pending(sequence)` — the fragment is not yet in `resolved`; a pending
/// slot with this sequence id has been queued in the registry. Call
/// [`TargetRegistry::flush_pending`] after further register calls to receive
/// the eventual value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolveOutcome {
    /// Immediately resolved — the fragment was in `resolved` or the URL was
    /// a non-fragment fallback (in which case the payload is empty).
    Resolved(String),
    /// Deferred — the sequence id addresses the queued pending slot;
    /// [`TargetRegistry::flush_pending`] will return the resolved value
    /// once the fragment lands.
    Pending(u32),
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
    /// Sequence id of the slot that was resolved on this flush — matches
    /// the value returned by the original [`ResolveOutcome::Pending`].
    pub sequence: u32,
    /// Resolved value; `Some(...)` on successful resolve. Reserved `None`
    /// for the future "give up after N passes" streaming exit — see the
    /// type-level note.
    pub value: Option<String>,
}

impl TargetRegistry {
    /// Construct an empty `TargetRegistry` (canonical zero-arg constructor,
    /// stable across the promotion from the M1.1 opaque placeholder).
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a resolved target — called from the register-site directive
    /// walker (bd raikiri-spike-96u.4) once an element with an `id`
    /// attribute is fully seen.
    ///
    /// **First-wins on duplicate fragment id.** Uses `entry().or_insert` so
    /// a second `register(id, info)` for the same fragment is a no-op. This
    /// matches the DOM id-resolution model: `getElementById` and every
    /// spec'd id-based lookup resolves to the **first element in tree
    /// order** when multiple elements share an id (which is invalid HTML in
    /// the first place — HTML §3.2.6.1 "id attribute" — but the resolution
    /// behavior is still well-defined). The register-site walker (M6, bd
    /// raikiri-spike-96u.4) is expected to invoke `register()` in tree
    /// order, so `or_insert` preserves the spec's first-in-tree-order
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
            let sequence = self.next_sequence();
            self.pending_slots.push(TargetSlot {
                sequence,
                fragment_id: key,
                request,
            });
            ResolveOutcome::Pending(sequence)
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
                    sequence: slot.sequence,
                    value: Some(info.resolve(&slot.request)),
                });
            } else {
                remaining.push(slot);
            }
        }
        self.pending_slots = remaining;
        resolutions
    }

    fn next_sequence(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        seq
    }
}

/// Drive a [`TargetRegistry`] from a
/// [`raikiri_style::property::ContentComponent`] — the "working conversion
/// path" for the M6 directive-apply pass.
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
/// [`CounterStyle::Decimal`] and the subset of CSS Counter Styles L3 §6
/// "Simple Predefined Counter Styles"
/// <https://www.w3.org/TR/css-counter-styles-3/#predefined-counters>
/// implemented here so far (enumerated on **bd raikiri-spike-og2**, P4,
/// scope/dom, discovered-from:96u.2) — `decimal-leading-zero`,
/// `lower-roman` / `upper-roman`, `lower-alpha` / `upper-alpha` (+ the
/// `lower-latin` / `upper-latin` aliases §6 defines with identical symbol
/// tables), `disc` / `circle` / `square`. ("Subset" here means
/// implementation coverage, not a registry boundary: every §6/§7
/// predefined style is a UA-stylesheet rule, not an author
/// `@counter-style` registration, so none of them need a registry —
/// implemented or not. Cf. reason 2 below, which scopes the registry
/// dependency to author-defined custom idents.)
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
///    parser/registry to resolve it against (og2 comment 2026-07-27 scope
///    (b); follow-up bd raikiri-spike-r7r1).
/// 3. **A §6/§7 predefined style this file hasn't implemented yet** — e.g.
///    `armenian`, `georgian`, `hebrew`, `lower-greek`, `cjk-decimal`,
///    `disclosure-open` / `disclosure-closed`. These are *not* unknown to
///    the spec — §6's lead paragraph is normative ("This stylesheet is
///    normative—UAs must include it in their UA stylesheet"), and §7 is
///    likewise a normative section (no informative-section marker
///    applies to it) — and none of them need a registry. Landing on
///    decimal here is an intentional current scope limit, not the
///    generate-a-counter step-1 path; tracked as bd raikiri-spike-ce3k
///    (discovered-from bd raikiri-spike-nu9z).
fn format_counter(value: i32, style: &CounterStyle) -> String {
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
/// (see [`format_counter`] doc for the full implemented set, the §6
/// anchor, and the three distinct reasons an unmatched name falls back to
/// `decimal`).
/// Matching is ASCII case-insensitive, consistent with how
/// `raikiri_style::property`'s parser already treats the `decimal` keyword
/// case-insensitively — CSS keyword idents are case-insensitive generally,
/// this file just extends the same rule to the rest of §6.
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
    } else {
        // Unmatched named style — decimal fallback for one of the three
        // reasons enumerated in the [`format_counter`] doc (genuinely
        // unknown / unreachable custom @counter-style / not-yet-implemented
        // §6-§7 predefined style).
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
    let mut remaining = value;
    let mut out = String::new();
    for &(weight, symbol) in &ROMAN_ADDITIVE {
        if weight > remaining {
            continue;
        }
        let reps = remaining / weight;
        for _ in 0..reps {
            out.push_str(symbol);
        }
        remaining -= weight * reps;
        if remaining == 0 {
            break;
        }
    }
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

/// Join a nested counter stack with `separator`, formatting each level via
/// [`format_counter`].
fn join_counter_stack(stack: &[i32], separator: &str, style: &CounterStyle) -> String {
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

    // ── Skeleton pins (carried from bd raikiri-spike-96u.1) ─────────

    #[test]
    fn target_registry_default_is_empty() {
        // Constructibility pin: `TargetRegistry::default()` yields an empty
        // registry. The M6 directive-apply driver (96u.4) will consume this
        // constructor.
        let reg = TargetRegistry::default();
        assert!(reg.resolved.is_empty());
        assert!(reg.pending_slots.is_empty());
        assert_eq!(reg.next_sequence, 0);
    }

    #[test]
    fn target_registry_shape_matches_design_7_2() {
        // Canonical shape pin (design §7.2, lines 1961-1964):
        //   resolved: HashMap<Symbol, TargetInfo>
        //   pending_slots: Vec<TargetSlot>
        // If this test breaks, the type has drifted from the design and the
        // coordinator should reconcile before landing follow-up work.
        let mut reg = TargetRegistry::default();
        reg.resolved
            .insert(Symbol::new("fragment-1"), TargetInfo::default());
        reg.pending_slots.push(TargetSlot {
            sequence: 0,
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
        // (join-with-sep of a one-element [0]) — NOT the empty string.
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
        let seq_a = match out_a {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
        };
        assert_eq!(seq_a, 0);
        assert_eq!(reg.pending_slots.len(), 1);

        // Second forward reference to a different fragment / kind.
        let out_b = reg.resolve_target_text("#chapter-3", ContentPart::Content);
        let seq_b = match out_b {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending, got Resolved"),
        };
        assert_eq!(seq_b, 1);
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
                sequence: seq_a,
                value: Some("3".to_owned()),
            }
        );
        assert_eq!(
            resolutions[1],
            PendingResolution {
                sequence: seq_b,
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
        // called in tree order by the M6 register-site walker; last-wins
        // would cause target-* to resolve against a later duplicate.
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
        let seq_a =
            match reg.resolve_target_counter("#a", Symbol::new("chapter"), CounterStyle::Decimal) {
                ResolveOutcome::Pending(s) => s,
                ResolveOutcome::Resolved(_) => panic!("expected Pending"),
            };
        let seq_b = match reg.resolve_target_counters(
            "#b",
            Symbol::new("section"),
            "-",
            CounterStyle::Decimal,
        ) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        let seq_c = match reg.resolve_target_text("#c", ContentPart::Content) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        let seq_a2 = match reg.resolve_target_text("#a", ContentPart::Before) {
            ResolveOutcome::Pending(s) => s,
            ResolveOutcome::Resolved(_) => panic!("expected Pending"),
        };
        assert_eq!((seq_a, seq_b, seq_c, seq_a2), (0, 1, 2, 3));
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
                sequence: seq_a,
                value: Some("7".to_owned()),
            }
        );
        assert_eq!(
            resolutions[1],
            PendingResolution {
                sequence: seq_a2,
                value: Some("prefix".to_owned()),
            }
        );

        // Retained slots keep their original ordering (seq 1 before seq 2).
        assert_eq!(reg.pending_slots[0].sequence, seq_b);
        assert_eq!(reg.pending_slots[1].sequence, seq_c);

        // Registering #b later resolves it on the next flush; #c stays.
        reg.register(Symbol::new("b"), make_info(&[("section", &[1, 2])], &[]));
        let resolutions2 = reg.flush_pending();
        assert_eq!(resolutions2.len(), 1);
        assert_eq!(resolutions2[0].sequence, seq_b);
        assert_eq!(resolutions2[0].value.as_deref(), Some("1-2"));
        assert_eq!(reg.pending_slots.len(), 1);
        assert_eq!(reg.pending_slots[0].sequence, seq_c);
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

    // ── ContentComponent wire-through (M5 → M6 conversion path) ─────

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
