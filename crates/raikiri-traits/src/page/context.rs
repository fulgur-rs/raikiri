//! `PageContext` — GCPM Phase B runtime state (design §7.0/§7.1/§7.2).
//!
//! **Promotion history.** Design §7.2
//! (`docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` lines
//! 1940-1965) gives `PageContext`'s canonical 6-field shape. Previously this
//! was an empty `#[non_exhaustive]` placeholder while the walk algorithm
//! (nested counter scopes, named-string 4-snapshot timing, running-binding
//! rebind) was built and unit tested dom-locally, isolated from raikiri-traits,
//! at what was then a `raikiri-dom`-local `PhaseBWalkState` type: populating
//! this struct's fields required new raikiri-traits `pub` accessor/mutator
//! methods, a cross-crate API-surface addition. This module is that
//! promotion:
//!
//! 1. The 4 `HashMap`-shaped fields (`counters` / `strings` / `running` /
//!    `targets`) are encapsulated behind the single
//!    [`PageContext::apply_directive`] entry point — no per-field
//!    accessor/mutator methods for these 4 (read accessors are narrower and
//!    separate, see below).
//! 2. `page_name` is a plain `pub` field — a copy-cheap scalar with no
//!    invariant to protect. `page_index` is also `pub` (direct writes still
//!    compile and are still supported), but [`PageContext::begin_page`] is
//!    now the coordinated way to advance it: it updates `page_index` *and*
//!    the `targets` registry's own page-local slot-sequence numbering
//!    together, so the two never drift apart on a page transition — see
//!    that method's doc for the full contract.
//!
//! **Type-promotion design decision**: the dom-local
//! `CounterStack` / `NamedStringState` *shapes* were promoted (moved) here
//! from raikiri-dom's own dom-local mirror, matching the `super::target`
//! precedent (`TargetRegistry` / `TargetInfo` canonical impl lives in
//! raikiri-traits, raikiri-dom only hosts producers). That dom-local mirror
//! was initially left in place after this promotion landed, since integration a
//! real DOM-tree-walking driver against it — `CounterStack::pop_scope` /
//! `NamedStringState` page-boundary call sites — was out of this
//! promotion's scope. The dom-local mirror has since been deleted; no
//! DOM-tree-walking driver in this repository drives this type yet.
//!
//! **Field-shape divergence from the dom-local mirror, now resolved**: the
//! dom-local `NamedStringState` stores `Option<StringSnapshot>` (a
//! `ContentSource` + a raw counter-values snapshot) instead of design §7.2's
//! canonical `Option<String>`, because the dom-local mirror could not reach
//! `super::target`'s private `format_counter` / `join_counter_stack`
//! helpers (a documented gap). Now that this promotion lives *inside*
//! raikiri-traits, that gap dissolves for the `Named`-counter-style half of
//! it: [`NamedStringState`] here matches the design doc's `Option<String>`
//! shape exactly, and `resolve_content_source` does eager resolution at
//! [`PageContext::apply_directive`] time using those two helpers (widened
//! from private to `pub(crate)` — see that fn's doc for why `pub(crate)`
//! rather than full `pub`: [`PageContext`] and `super::target` are sibling
//! `page` submodules, so a same-crate, narrower-than-`pub` visibility
//! already suffices; there is no cross-crate need) *when the content-list is
//! fully resolvable* — otherwise it skips the assignment (never fabricates
//! `Some("")`), preserving the dom-local mirror's fail-closed discipline for
//! the DOM-element-needing half of the original gap (`Attr`/`Content`/etc.
//! — see `resolve_content_source`'s doc "Skipping, not fabricating").
//!
//! **`RegisterTarget` now really mutates `self.targets`** — unlike
//! the dom-local mirror's documented no-op (it never owned
//! `TargetRegistry` at all). See [`PageContext::apply_directive`]'s doc for
//! what it can and cannot populate from a bare directive, and
//! [`PageContext::set_targets`] for the complementary bulk-integration path the
//! register-site walker uses.

use std::collections::HashMap;

use super::target::{format_counter, join_counter_stack};
use super::{
    ContentSource, ContentValueItem, GcpmDirective, RunningTemplateId, TargetInfo, TargetRegistry,
};
use crate::dom::Symbol;

// ── CounterStack ─────────────────────────────────────────────────────────

/// Nested-scope stack for one named counter (design §7.2's `CounterStack`,
/// "support nested scopes with a stack" — CSS Lists 3 §4
/// <https://www.w3.org/TR/css-lists-3/#auto-numbering>).
///
/// Each `counter-reset` on a descendant element establishes a *new* counter
/// instance nested inside any ancestor instance of the same name (§4.1);
/// `counter-increment` / `counter-set` mutate the innermost (most deeply
/// nested) instance currently in scope (§4.2). Representing that as a stack
/// — reset = push a new frame, increment/set = mutate the top frame — is the
/// natural fit design §7.2 names.
///
/// Promoted from what was raikiri-dom's own dom-local `CounterStack`
/// (since deleted there) — same algorithm, now `pub` (design §7.2 gives `PageContext.counters:
/// HashMap<Symbol, CounterStack>` with no `pub` on the field itself, so the
/// *value type* returned by [`PageContext::counter`] must be nameable from
/// outside this crate).
///
/// **Caller invariant — scope exit is the caller's responsibility.**
/// [`Self::reset`] unconditionally *pushes* a new frame; it never replaces
/// the innermost one. Popping that frame when the walk leaves the
/// originating element's subtree is **not** done automatically here —
/// [`Self::pop_scope`] exists for a DOM-tree-driven walker to call at the
/// right point (subtree exit, matching the real CSS scoping rule where two
/// *sibling* elements each resetting the same counter get independent,
/// same-depth scopes, not accumulating nesting). Such a walker calls
/// [`PageContext::pop_counter_scope`] at the resetting element's own
/// parent's subtree exit. CSS Lists 3 §4.3's separate "obscuring" rule uses
/// the same [`Self::pop_scope`] primitive: a later sibling's reset of the
/// same name is specified to evict an earlier sibling's still-open frame
/// outright rather than merely sit above it, so the walker pops the earlier
/// sibling's frame right before applying the later sibling's own reset,
/// instead of leaving both frames on the stack for [`Self::values`]'s
/// callers to see joined together. `raikiri_dom`'s counter-snapshot walk
/// applies the same scope model to its own stacks. [`Self::reset`] itself still only ever pushes,
/// never replaces in place; the eviction is the caller's own extra
/// `pop_scope` call, not a change to this method.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct CounterStack {
    /// Nested scope frames, outermost first. Empty = the counter has not
    /// been instantiated yet (CSS Lists 3 §4.2: an un-instantiated counter
    /// behaves as absent until reset/increment/set creates one).
    frames: Vec<i32>,
}

impl CounterStack {
    /// `counter-reset: name value` (CSS Lists 3 §4.1) — push a new nested
    /// scope frame at `value`. See the type-level "Caller invariant" note:
    /// always pushes, never replaces the innermost frame.
    pub fn reset(&mut self, value: i32) {
        self.frames.push(value);
    }

    /// `counter-increment: name delta` (CSS Lists 3 §4.2
    /// <https://www.w3.org/TR/css-lists-3/#propdef-counter-increment>) — add
    /// `delta` to the innermost frame. An un-instantiated counter (empty
    /// stack) is implicitly created at `delta` — the spec's "increment a
    /// non-existent counter" path instantiates it at 0 and then applies the
    /// increment, which is equivalent to starting the new frame at `delta`
    /// directly.
    ///
    /// **Saturating, not wrapping/panicking, on overflow**:
    /// `counter-reset: c 2147483647` followed by `counter-increment: c 1` is
    /// spec-legal CSS (CSS Lists 3 places no range limit on
    /// `<integer>`/`<counter-name>` values) and would panic on `+=`'s debug
    /// overflow check — or silently wrap in release, which is worse (a
    /// rendered counter value jumping to `i32::MIN`). `i32::saturating_add`
    /// clamps to `i32::MAX`/`i32::MIN` instead, matching the "fail-closed,
    /// not fail-silent-wrong" discipline this crate already applies
    /// elsewhere (principle 3). A second, lower-severity finding (unbounded frame
    /// growth via unbounded nested `counter-reset`) is tracked separately,
    /// gated behind that same not-yet-built driver — no fix needed here.
    pub fn increment(&mut self, delta: i32) {
        match self.frames.last_mut() {
            Some(top) => *top = top.saturating_add(delta),
            None => self.frames.push(delta),
        }
    }

    /// `counter-set: name value` (CSS Lists 3 §4.2
    /// <https://www.w3.org/TR/css-lists-3/#propdef-counter-set>) — set the
    /// innermost frame to `value`. An un-instantiated counter is implicitly
    /// created at `value` (mirrors [`Self::increment`]'s implicit-create).
    pub fn set(&mut self, value: i32) {
        match self.frames.last_mut() {
            Some(top) => *top = value,
            None => self.frames.push(value),
        }
    }

    /// Exit the innermost nested scope, returning its value (`None` if the
    /// stack was already empty). See the type-level "Caller invariant" note
    /// for when a DOM-tree walker calls this (via
    /// [`PageContext::pop_counter_scope`]'s forwarding call).
    pub fn pop_scope(&mut self) -> Option<i32> {
        self.frames.pop()
    }

    /// The innermost (currently effective) value, if any scope exists.
    pub fn current(&self) -> Option<i32> {
        self.frames.last().copied()
    }

    /// Every nested frame, outermost first — the shape `counters()`
    /// (CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>)
    /// joins with a separator when rendering.
    pub fn values(&self) -> &[i32] {
        &self.frames
    }
}

// ── NamedStringState ─────────────────────────────────────────────────────

/// `string-set` 4-snapshot state machine (design §7.2's `NamedStringState`;
/// CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set> /
/// §1.1.2 <https://www.w3.org/TR/css-gcpm-3/#string-first>).
///
/// Promoted from what was raikiri-dom's own dom-local `NamedStringState`
/// (since deleted there) — same 4-snapshot
/// timing rules, but with the
/// design-doc-canonical `Option<String>` field type (fully resolved text)
/// instead of that dom-local mirror's `Option<StringSnapshot>` deferral; see
/// module-level doc "Field-shape divergence" for why this promotion can do
/// what the dom-local mirror could not.
///
/// **4-snapshot field mapping** — design §7.2's prose names the 4 snapshots
/// "start/first/last/first-except" (the CSS GCPM 3 §1.1.2 keyword set) but
/// the struct itself has `on_page_start` / `on_page_first_use` /
/// `on_page_last_use` / `running` (no `first_except` field). Following the
/// struct (the normative part) rather than the prose: `first-except` is
/// derivable at read time from `on_page_first_use` plus page-position
/// context (spec: same as `first`, except empty on the page where a
/// `string-set` assignment for this name actually occurs) rather than stored
/// as its own snapshot slot — deliberate, not an oversight, carried over
/// unchanged from the dom-local mirror's own doc.
///
/// **None of the 4 keywords reduce to a single stored field read** — a
/// future `string()` implementation must combine these fields with
/// page-position context for 2 of the 4 keywords, not just `first-except`:
/// `first` → `on_page_first_use`, else `on_page_start` (CSS GCPM 3
/// §1.1.2: "the value of the first assignment on the page ... If there is no
/// assignment on the page, the entry value is used"). `start` → **if the
/// querying element is the page's first formatted element**,
/// `on_page_first_use`; **otherwise** `on_page_start` — this element-position
/// condition is *not* captured by this state machine at all and must come
/// from the caller. `last` → `on_page_last_use`, else `on_page_start` (exit
/// value). `first-except` → `on_page_first_use.is_some()` ? empty :
/// `on_page_start` (condition is page-local, not element-position). Carried
/// over unchanged from the dom-local mirror (since deleted from raikiri-dom)
/// — the consuming `string()` implementation itself is a future consumer's
/// task, not this promotion's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct NamedStringState {
    /// The value in effect at the start of the page — `running` as it stood
    /// when [`Self::begin_page`] was last called (CSS GCPM 3 §1.1.2's "entry
    /// value": the value in effect at the end of the previous page, carried
    /// forward via `running`). `None` before any `string-set` has ever fired
    /// in the document.
    on_page_start: Option<String>,
    /// The first `string-set` assignment applied since the current page
    /// began. `None` until the first assignment on this page.
    on_page_first_use: Option<String>,
    /// The most recent `string-set` assignment applied since the current
    /// page began. `None` until the first assignment on this page;
    /// thereafter tracks every subsequent assignment (including the first).
    on_page_last_use: Option<String>,
    /// The always-current value — every `string-set` application overwrites
    /// this immediately, independent of page boundaries.
    /// [`Self::begin_page`] reads this to seed the next page's
    /// `on_page_start`.
    running: Option<String>,
}

impl NamedStringState {
    /// Page-boundary hook — snapshot the current `running` value as the new
    /// page's entry/start value, and clear the per-page first/last-use
    /// trackers (a fresh page starts with no assignments of its own).
    ///
    /// [`PageContext::begin_page`] calls this; no production driver in this
    /// repository calls either yet.
    pub fn begin_page(&mut self) {
        self.on_page_start = self.running.clone();
        self.on_page_first_use = None;
        self.on_page_last_use = None;
    }

    /// Apply a `string-set` directive's already-resolved text: always
    /// updates `running`; updates `on_page_first_use` only if this is the
    /// first assignment since the last [`Self::begin_page`] (or since
    /// construction, if `begin_page` has never run); always updates
    /// `on_page_last_use`.
    ///
    /// Module-private — mutation of a tracked [`NamedStringState`] only
    /// happens via [`PageContext::apply_directive`] (the human-decided
    /// single entry point), never directly from outside this crate.
    fn apply_string_set(&mut self, resolved: String) {
        if self.on_page_first_use.is_none() {
            self.on_page_first_use = Some(resolved.clone());
        }
        self.on_page_last_use = Some(resolved.clone());
        self.running = Some(resolved);
    }

    /// The page-entry value (see type-level "4-snapshot field mapping").
    pub fn on_page_start(&self) -> Option<&str> {
        self.on_page_start.as_deref()
    }

    /// The first assignment on the current page, if any.
    pub fn on_page_first_use(&self) -> Option<&str> {
        self.on_page_first_use.as_deref()
    }

    /// The most recent assignment on the current page, if any.
    pub fn on_page_last_use(&self) -> Option<&str> {
        self.on_page_last_use.as_deref()
    }

    /// The always-current value, independent of page boundaries.
    pub fn running(&self) -> Option<&str> {
        self.running.as_deref()
    }
}

/// Fully resolve a `string-set` `<content-list>` ([`ContentSource`]) into
/// text, evaluated against `counters`' *current* state at call time —
/// [`PageContext::apply_directive`] calls this synchronously the instant a
/// `StringSet` directive is applied, so the counter values baked into the
/// result are automatically "at assignment time" (CSS GCPM 3 §1.1.1: "The
/// content values of named strings are assigned at the point when the
/// content box of the element is first created") without needing to store a
/// separate raw snapshot the way the pre-promotion dom-local mirror did (see
/// module-level doc "Field-shape divergence").
///
/// **Only [`ContentValueItem::Literal`] / `Counter` / `Counters` are
/// resolvable here — `None` (skip the assignment) if *any* item in the
/// content-list is not one of those three.** Every other variant
/// (`String`/`Element`/`Content`/`Attr`/`TargetCounter`/`TargetCounters`/
/// `TargetText`/`Image`/`Contents`/`Quote`/`Leader`) needs context this
/// directive-only resolution path structurally cannot have: DOM element
/// access (`String`/`Element`/`Content`/`Attr`), mutable [`TargetRegistry`]
/// access for a second resolve pass (`TargetCounter`/`TargetCounters`/
/// `TargetText` — and [`PageContext::apply_directive`] already holds
/// `&mut self.targets` exclusively for the *current* directive, so
/// re-entrant resolution isn't available either), or paint-time layout
/// machinery (`Image`/`Contents`/`Quote`/`Leader`).
///
/// **Skipping, not fabricating `""`, is load-bearing.** [`NamedStringState`]
/// stores `Option<String>`; `None` and `Some("")` are *not* interchangeable
/// under CSS GCPM 3 §1.1.2's `string()` keyword rules — `first`/`start`
/// fall back to the entry value (`on_page_start`/`running` carried from the
/// previous page) precisely when no on-page assignment happened, and
/// `first-except` is defined by whether an assignment occurred at all, not
/// by what it resolved to. Storing `Some("")` for an unresolvable
/// content-list (e.g. `string-set: chapter attr(data-title)`, the canonical
/// GCPM running-header idiom) would suppress that fallback and make
/// `first-except` see a phantom assignment — silently and unrecoverably
/// wrong, since no downstream consumer can tell "resolved to empty" from
/// "couldn't resolve" once collapsed to the same `Some("")`. Returning
/// `None` here so the whole directive is skipped (see
/// [`PageContext::apply_directive`]'s `StringSet` arm) preserves the same
/// fail-closed discipline (principle 3) the pre-promotion dom-local mirror's
/// `Option<StringSnapshot>` deferral already established — this promotion
/// narrows *which* content-lists resolve (the `Named`-counter-style half of
/// that old gap is fixed, since `format_counter` is reachable now), it does
/// not change the "don't guess at wrong text" discipline itself.
fn resolve_content_source(
    source: &ContentSource,
    counters: &HashMap<Symbol, CounterStack>,
) -> Option<String> {
    let mut out = String::new();
    for item in &source.items {
        match item {
            ContentValueItem::Literal(s) => out.push_str(s),
            ContentValueItem::Counter { name, style } => {
                // Absent counter → 0 (CSS Lists 3 §4.2 default-init), same
                // fallback super::target::TargetInfo::resolve uses.
                let value = counters
                    .get(name)
                    .and_then(CounterStack::current)
                    .unwrap_or(0);
                out.push_str(&format_counter(value, style));
            }
            ContentValueItem::Counters {
                name,
                separator,
                style,
            } => {
                let stack: &[i32] = counters.get(name).map(CounterStack::values).unwrap_or(&[]);
                if stack.is_empty() {
                    // Absent or empty-stack counter → single formatted "0",
                    // not "" (same rule as TargetInfo::resolve's Counters
                    // arm — CSS Content 3 §2.6.2's initial-value-0 default).
                    out.push_str(&format_counter(0, style));
                } else {
                    out.push_str(&join_counter_stack(stack, separator, style));
                }
            }
            ContentValueItem::String { .. }
            | ContentValueItem::Element { .. }
            | ContentValueItem::Content { .. }
            | ContentValueItem::Attr { .. }
            | ContentValueItem::TargetCounter { .. }
            | ContentValueItem::TargetCounters { .. }
            | ContentValueItem::TargetText { .. }
            | ContentValueItem::Image { .. }
            | ContentValueItem::Contents
            | ContentValueItem::Quote(_)
            | ContentValueItem::Leader(_) => {
                // Unresolvable in this context — skip the whole assignment
                // rather than fabricate empty text. See this fn's doc
                // "Skipping, not fabricating" for why that distinction is
                // load-bearing.
                return None;
            }
        }
    }
    Some(out)
}

// ── PageContext ──────────────────────────────────────────────────────────

/// GCPM Phase B runtime state — the mutable page-local state a Phase B walk
/// (design §7.0 "runtime side") builds up as it applies [`GcpmDirective`]s
/// and reads back for `content` resolution.
///
/// See module-level doc for the promotion history and the design decision
/// this shape implements:
///
/// - `counters` / `strings` / `running` / `targets` are private, mutated
///   *only* via [`Self::apply_directive`] (single entry point — no
///   per-field accessor/mutator) with one narrow exception:
///   [`Self::pop_counter_scope`], a scope-exit-only mutation no
///   `GcpmDirective` variant can express — see that method's doc for why.
///   Otherwise read via the narrow accessors below.
/// - `page_index` / `page_name` are plain `pub` fields (copy-cheap scalars,
///   no invariant to protect).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
#[doc(hidden)]
pub struct PageContext {
    /// Counter name → nested scope stack (design §7.2, CSS Lists 3 §4).
    counters: HashMap<Symbol, CounterStack>,
    /// Named-string identifier → 4-snapshot state (design §7.2, CSS GCPM 3
    /// §1.1.1/§1.1.2).
    strings: HashMap<Symbol, NamedStringState>,
    /// Running-template name → currently bound template (design §7.2, CSS
    /// GCPM 3 §1.2.1 `position: running(name)`).
    running: HashMap<Symbol, RunningTemplateId>,
    /// `target-*()` / `element()` runtime registry (design §7.2). Wired
    /// either incrementally via [`Self::apply_directive`]'s `RegisterTarget`
    /// arm, or in bulk via [`Self::set_targets`] — see both methods' docs.
    targets: TargetRegistry,
    /// 0-based index of the page this context describes (design §7.2).
    /// Plain `pub` field, directly settable — but [`Self::begin_page`] is
    /// the preferred way to advance it: it updates this field *and* the
    /// `targets` registry's own page-local slot-sequence numbering together
    /// (design §7.2 names `PageContext` as sole owner of both `page_index`
    /// and `targets` as sibling fields). A direct write here leaves the
    /// registry's sequence numbering un-advanced — see [`Self::begin_page`]'s
    /// doc for what that desync looks like.
    pub page_index: u32,
    /// `@page` named-page association in effect for this page, if any
    /// (design §7.2). Plain `pub` field, copy-cheap scalar with no
    /// invariant to protect — unlike `page_index`, nothing else in this
    /// context derives from `page_name`, so there is no coordinating method
    /// for it.
    pub page_name: Option<Symbol>,
}

impl PageContext {
    /// Construct an empty `PageContext` (stable across the placeholder →
    /// populated-struct promotion — `crates/raikiri/tests/external_consumer.rs` and
    /// `crates/raikiri-traits/src/strategy.rs`'s `TargetResolver` already
    /// depend on this constructor and `Default` continuing to work).
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one `GcpmDirective` to this context — the single entry point
    /// for mutating `counters` / `strings` / `running` / `targets` (a
    /// deliberate design decision). Mirrored the dispatch of what was
    /// raikiri-dom's own dom-local `PhaseBWalkState::apply_directive`
    /// (since deleted there), with two differences documented on the
    /// relevant arms below:
    /// `StringSet` resolves text where possible and otherwise skips the
    /// assignment (module doc "Field-shape divergence",
    /// `resolve_content_source`'s doc "Skipping, not fabricating") and
    /// `RegisterTarget` really registers (module doc "`RegisterTarget` now
    /// really mutates `self.targets`").
    ///
    /// **Exhaustive same-crate match, no wildcard arm.** `GcpmDirective` is
    /// defined in this crate; `#[non_exhaustive]`'s "downstream `match` needs
    /// a wildcard" restriction is a cross-crate rule, so unlike the
    /// dom-local mirror's necessarily-wildcarded copy (that copy lived in a
    /// different crate; it has since been deleted), this match lists
    /// every variant — a future 7th variant fails to compile right here
    /// instead of silently no-op'ing under a `_` arm.
    pub fn apply_directive(&mut self, directive: &GcpmDirective) {
        match directive {
            GcpmDirective::CounterIncrement { name, delta } => {
                self.counters
                    .entry(name.clone())
                    .or_default()
                    .increment(*delta);
            }
            GcpmDirective::CounterReset { name, value } => {
                self.counters.entry(name.clone()).or_default().reset(*value);
            }
            GcpmDirective::CounterSet { name, value } => {
                self.counters.entry(name.clone()).or_default().set(*value);
            }
            GcpmDirective::StringSet { name, source } => {
                // Resolve against *current* counter state now — see
                // resolve_content_source's doc for why "now" is correct
                // ("at assignment time") without a separate raw snapshot.
                // `None` (an unresolvable content-list item) skips the
                // assignment entirely — no snapshot slot is fabricated, and
                // any prior tracked state for `name` is left untouched (see
                // resolve_content_source's doc "Skipping, not fabricating").
                //
                // Convention departure: `super::target`'s
                // `ResolveOutcome::Pending` pattern makes "can't resolve yet"
                // observable to the caller; this arm's silent skip does not
                // — `apply_directive` returns `()`, so there is no signal
                // that a StringSet was dropped. Left as-is deliberately (no
                // driver exists yet to consume a richer return type —
                // building one now would be speculative), but this is where
                // the `attr()`/`content()` gap would need reconsidering if a
                // future consumer needs to observe silently-dropped
                // assignments.
                if let Some(resolved) = resolve_content_source(source, &self.counters) {
                    self.strings
                        .entry(name.clone())
                        .or_default()
                        .apply_string_set(resolved);
                }
            }
            GcpmDirective::RegisterRunning { name, template_id } => {
                // Insert-or-overwrite — a later RegisterRunning for the same
                // name rebinds it to the new template_id (last-wins).
                self.running.insert(name.clone(), *template_id);
            }
            GcpmDirective::RegisterTarget { fragment_id } => {
                // Best-effort registration from directive context alone: a
                // bare RegisterTarget carries only `fragment_id`, so the
                // TargetInfo built here can only carry *this context's live
                // counters* snapshot (mirrors the counter-freeze precedent
                // StringSet already uses) — text_parts is left empty, since
                // DOM element access isn't available at this call site.
                //
                // Unlike resolve_content_source (which SKIPS an
                // unresolvable string-set entirely, see that fn's doc), this
                // arm still registers with text_parts empty rather than
                // skipping registration altogether: TargetInfo::text_part's
                // own documented "" fallback for absent parts is pre-existing
                // blessed semantics (CSS Content 3 §2.6.3 is itself marked
                // not-ready-for-implementation on this point). The
                // counters snapshot registered here is independently useful
                // on its own. A caller wanting real text must use
                // `Self::set_targets` instead — see the call-order note
                // below for why that means "before", not "after", this
                // directive fires. `resolve_target_text` on a fragment
                // registered only through this arm therefore returns
                // `Resolved("")`, not `Pending` — a known, accepted
                // imprecision (widening `ResolveOutcome` to distinguish
                // "resolved empty" from "text genuinely unavailable" is
                // future, out-of-scope work).
                //
                // TargetRegistry::register is first-wins (entry().or_insert)
                // by design, so IF set_targets already populated this
                // fragment_id with a fuller TargetInfo (text included)
                // BEFORE this directive is applied, this arm's counts-only
                // registration is naturally a no-op for that fragment — no
                // clobbering *in that call order*. The reverse order is NOT
                // safe: set_targets is a wholesale replace (see its own
                // doc), so calling it AFTER this arm has registered
                // fragments discards every one of them, not just this
                // fragment_id. Callers driving both paths (a future
                // DOM-tree-walking driver) must call set_targets first.
                let counters = self
                    .counters
                    .iter()
                    .map(|(k, v)| (k.clone(), v.values().to_vec()))
                    .collect();
                let info = TargetInfo {
                    counters,
                    ..TargetInfo::default()
                };
                self.targets.register(fragment_id.clone(), info);
            }
        }
    }

    /// Pop the innermost nested scope tracked under `name`, returning its
    /// value (`None` if `name` has no tracked counter at all, or if the
    /// tracked `CounterStack` is already empty). A thin forwarding
    /// wrapper around [`CounterStack::pop_scope`] — this method exists
    /// because `counters` has no `counter_mut` accessor
    /// ([`Self::apply_directive`]'s doc "single entry point" explains why
    /// there is deliberately no per-field mutable accessor for the 4
    /// encapsulated fields) and none of `GcpmDirective`'s existing six
    /// variants carries a "pop a scope" instruction.
    ///
    /// **Why a caller needs this.** CSS Lists 3 §4.3
    /// <https://www.w3.org/TR/css-lists-3/#auto-numbering> scopes a
    /// `counter-reset` to "the element's descendants and its following
    /// siblings with their descendants" — the pushed scope must stay visible
    /// not only to the resetting element's descendants but also to its
    /// following siblings (and their descendants) within the same parent. A
    /// document-order tree walk that pushes a new scope (via
    /// [`Self::apply_directive`]'s `CounterReset` arm) when entering the
    /// resetting element must therefore *not* pop that scope again when
    /// leaving that same element — doing so would only keep the scope
    /// visible to its descendants, losing the following-sibling half of the
    /// rule. The correct point to pop is when the resetting element's
    /// *parent's* subtree walk finishes, once every following sibling has
    /// had its turn. This method is that pop, exposed for a caller outside
    /// this crate to invoke at the right point in its own walk.
    pub fn pop_counter_scope(&mut self, name: &Symbol) -> Option<i32> {
        self.counters.get_mut(name)?.pop_scope()
    }

    /// Page-boundary hook — the coordinated way to advance to a new page.
    /// Updates three things together: `page_index` itself, every tracked
    /// `NamedStringState` (via [`NamedStringState::begin_page`]), and the
    /// `targets` registry's own page-local slot-sequence numbering (via
    /// [`TargetRegistry::begin_page`]). Design §7.2 names `PageContext` as
    /// sole owner of `page_index` and `targets` as sibling fields; this
    /// method is what actually keeps the two paired on a transition. No
    /// production caller exists yet.
    ///
    /// **The sole sanctioned way to advance `page_index` from here on.**
    /// `page_index` remains a directly-writable `pub` field (see its own
    /// doc) for callers with no need for registry coordination (e.g.
    /// constructing a `PageContext` for a single, already-known page), but a
    /// direct write during an in-progress multi-page walk leaves the
    /// registry's slot-sequence numbering stale — this method exists so a
    /// driver never has to hand-coordinate the two mechanisms itself.
    ///
    /// **Call order inside this method is registry-first, deliberately.**
    /// [`TargetRegistry::begin_page`] asserts `page_index` is monotonically
    /// non-decreasing (design §7.6) and panics otherwise — see its own doc
    /// "Panics". Calling it before mutating `self.page_index` / `self.strings`
    /// means a panic here leaves every field untouched (still paired with
    /// the *previous* page), rather than leaving `page_index` advanced while
    /// `targets` is stuck on the old page — exactly the desync this method
    /// exists to rule out.
    ///
    /// **Lifetime contract, otherwise unchanged from before this method
    /// coordinated `targets`**: `counters` carries forward across the page
    /// boundary untouched (CSS Lists 3 §4's counters are document-scoped,
    /// not reset per page), and `targets`'s own `resolved` / `pending_slots`
    /// maps persist across pages too (`TargetRegistry` is per-*document*,
    /// only its internal sequence numbering is page-local). This implies a
    /// single `PageContext` instance must live for the **whole document**,
    /// mutated in place across pages via repeated calls to this method —
    /// constructing a fresh `PageContext` per page would incorrectly
    /// discard `targets` and `counters` state that must persist.
    ///
    /// **`page_name` is not touched here** — nothing else in this context
    /// derives from it, so unlike `page_index` there is no coordinating
    /// obligation; callers still assign it directly (see its own field
    /// doc).
    ///
    /// **Idempotence asymmetry on a same-`page_index` call.**
    /// [`TargetRegistry::begin_page`] treats a same-index call as a no-op
    /// (redundant same-page calls don't corrupt an in-progress page's
    /// sequence count — see its own doc), but
    /// [`NamedStringState::begin_page`] is not similarly guarded: calling
    /// this method twice with the same `page_index` re-snapshots `running`
    /// into `on_page_start` and clears the first/last-use trackers a second
    /// time. A driver must still call this once per genuine page
    /// transition, not defensively on every dispatch.
    ///
    /// **Residual gap this method does not close: [`Self::set_targets`].**
    /// Calling `set_targets` *after* this method replaces `targets`
    /// wholesale with whatever page-index state the incoming
    /// `TargetRegistry` already carries — if that registry is freshly
    /// constructed (page index 0) or otherwise behind, the desync this
    /// method exists to prevent reappears, just through the bulk-replace
    /// path instead of a bare field write. The `RegisterTarget` arm of
    /// [`Self::apply_directive`] already documents a call-order convention
    /// requiring `set_targets` before directive-driven registration for an
    /// unrelated reason; this adds a second, page-boundary-shaped reason to
    /// call `set_targets` before, not after, this method's call for a given
    /// page.
    ///
    /// # Panics
    ///
    /// Panics if `page_index` is less than [`TargetRegistry`]'s own internal
    /// `page_index` — that is, `self.targets`'s private copy, not the
    /// `page_index` field on this struct — forwarded from
    /// [`TargetRegistry::begin_page`]'s monotonic invariant (design §7.6:
    /// `page_index` must advance in emitted order). See that method's doc
    /// "Panics" for the full rationale.
    ///
    /// This baseline coincides with "the value most recently passed to this
    /// method" only when no [`Self::set_targets`] call has intervened — see
    /// "Residual gap this method does not close" above. A `set_targets` call
    /// in between can leave the registry's internal `page_index` behind or
    /// ahead of what this method last saw, so a subsequent call here can
    /// panic (or fail to panic) against a baseline other than this method's
    /// own call history.
    pub fn begin_page(&mut self, page_index: u32) {
        // Registry first — its `assert!` runs before either of the other
        // two mutations below, so a panic here leaves this whole call a
        // no-op rather than a partially-applied page transition. See this
        // method's doc "Call order inside this method is registry-first".
        self.targets.begin_page(page_index);
        self.page_index = page_index;
        for state in self.strings.values_mut() {
            state.begin_page();
        }
    }

    /// Replace `targets` wholesale with a pre-built [`TargetRegistry`] — the
    /// bulk-integration path for a register-site walker that produces a
    /// complete registry (counters *and* descendant text) from a dedicated
    /// DOM walk rather than from a `GcpmDirective` stream. No such walker in
    /// this repository calls it yet; raikiri-traits owns the resulting data
    /// (design §7.0's "runtime side raikiri-dom / shared types
    /// raikiri-traits" split).
    ///
    /// Not itself an [`Self::apply_directive`] arm — `GcpmDirective` has no
    /// variant carrying a whole registry, only single-fragment
    /// `RegisterTarget`. `&mut self`, matching this type's other mutators,
    /// rather than a consuming builder.
    ///
    /// **Not a violation of the "no per-field accessor" rule** —
    /// `set_targets` is a deliberate, narrower exception:
    /// `counters`/`strings`/`running` are built up incrementally,
    /// directive-by-directive, and the "single apply_directive entry point"
    /// rule exists to protect `PageContext`'s internal accumulation
    /// semantics (counter-stack nesting, 4-snapshot timing) from being
    /// corrupted by ad-hoc external pokes. `TargetRegistry` is different in
    /// kind — it's a separately-produced, already-fully-encapsulated type
    /// (its own pub methods, its own invariants) built in bulk by the
    /// dom-side register-site walker, not incrementally accumulated by
    /// `PageContext` itself. A bulk transfer method for it doesn't create
    /// the same risk the no-per-field-accessor rule was written to prevent.
    /// Also narrower in purpose than a general mutable accessor like
    /// `targets_mut` (which [`Self::targets`]'s doc explains was rejected),
    /// since it only ever *replaces the whole value*, never exposes the live
    /// registry for arbitrary external mutation.
    pub fn set_targets(&mut self, targets: TargetRegistry) {
        self.targets = targets;
    }

    /// Read the counter stack tracked under `name`, if any counter
    /// directive has touched it.
    pub fn counter(&self, name: &Symbol) -> Option<&CounterStack> {
        self.counters.get(name)
    }

    /// Read the named-string 4-snapshot state tracked under `name`, if any
    /// `StringSet` directive has touched it.
    pub fn string_state(&self, name: &Symbol) -> Option<&NamedStringState> {
        self.strings.get(name)
    }

    /// Read the running-template binding currently bound to `name`, if any.
    pub fn running_binding(&self, name: &Symbol) -> Option<RunningTemplateId> {
        self.running.get(name).copied()
    }

    /// Read-only access to the `target-*()` / `element()` registry.
    ///
    /// No `targets_mut` companion: `targets` is one of the 4 fields the
    /// human decision encapsulates behind [`Self::apply_directive`] /
    /// [`Self::set_targets`] specifically to keep mutation single-entry —
    /// a mutable accessor would let a caller bypass both and mutate the
    /// registry directly, defeating that encapsulation.
    pub fn targets(&self) -> &TargetRegistry {
        &self.targets
    }
}

#[cfg(test)]
mod tests;
