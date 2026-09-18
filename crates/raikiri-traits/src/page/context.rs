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
//! promotion's scope. raikiri-dom's `crate::phase_b` driver has since been
//! built against this promoted type directly, and the dom-local mirror
//! (having no remaining caller once that driver existed) has been deleted.
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
/// "stack で nested scope 対応" — CSS Lists 3 §4
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
/// same-depth scopes, not accumulating nesting). `raikiri_dom::phase_b`'s
/// walker now does exactly this, via [`PageContext::pop_counter_scope`]'s
/// forwarding call at the resetting element's own parent's subtree exit.
/// `raikiri_dom::phase_b` also implements CSS Lists 3 §4.3's separate
/// "obscuring" rule this way, using the same [`Self::pop_scope`] primitive:
/// a later sibling's reset of the same name is specified to evict an
/// earlier sibling's still-open frame outright rather than merely sit above
/// it, so that walker calls [`Self::pop_scope`] (via
/// [`PageContext::pop_counter_scope`]) on the earlier sibling's frame right
/// before applying the later sibling's own reset, instead of leaving both
/// frames on the stack for [`Self::values`]'s callers to see joined
/// together — see `raikiri_dom::phase_b`'s module doc "Counter-scope exit"
/// for the full mechanism. [`Self::reset`] itself still only ever pushes,
/// never replaces in place; the eviction is the caller's own extra
/// `pop_scope` call, not a change to this method.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
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
    /// elsewhere (原則3). A second, lower-severity finding (unbounded frame
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
    /// — `raikiri_dom::phase_b`'s walker now calls this, via
    /// [`PageContext::pop_counter_scope`]'s forwarding call, at the
    /// resetting element's own parent's subtree exit.
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
    /// No production driver calls this yet — `raikiri_dom::phase_b`'s
    /// per-page walk now reaches the promoted state through
    /// [`PageContext::begin_page`], but that driver is still
    /// `#[allow(dead_code)]` and is exercised only by tests.
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
/// fail-closed discipline (原則3) the pre-promotion dom-local mirror's
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

    /// Apply one [`GcpmDirective`] to this context — the single entry point
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
    /// **Exhaustive same-crate match, no wildcard arm.** [`GcpmDirective`] is
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
                // not-ready-for-implementation on this point), and
                // `raikiri_dom::target::build_target_registry` already ships
                // with 3 of 4 ContentPart variants absent by design (module
                // doc "Text parts — ContentPart::Content only"). The
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
    /// tracked [`CounterStack`] is already empty). A thin forwarding
    /// wrapper around [`CounterStack::pop_scope`] — this method exists
    /// because `counters` has no `counter_mut` accessor
    /// ([`Self::apply_directive`]'s doc "single entry point" explains why
    /// there is deliberately no per-field mutable accessor for the 4
    /// encapsulated fields) and none of [`GcpmDirective`]'s existing six
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
    /// [`NamedStringState`] (via [`NamedStringState::begin_page`]), and the
    /// `targets` registry's own page-local slot-sequence numbering (via
    /// [`TargetRegistry::begin_page`]). Design §7.2 names `PageContext` as
    /// sole owner of `page_index` and `targets` as sibling fields; this
    /// method is what actually keeps the two paired on a transition. No
    /// production caller exists yet — `raikiri_dom::phase_b`'s per-page walk
    /// now calls this method, but that driver is still
    /// `#[allow(dead_code)]` and is exercised only by tests.
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
    /// bulk-integration path for
    /// `raikiri_dom::target::build_target_registry`'s register-site walker,
    /// which produces a complete registry (counters *and* descendant text)
    /// from a dedicated DOM walk rather than from a `GcpmDirective` stream
    /// (see that function's module doc — `RegisterTarget` is synthesized,
    /// not consumed, there). raikiri-dom drives (runs the walk, calls this
    /// setter); raikiri-traits owns the resulting data (design §7.0's
    /// "runtime side raikiri-dom / shared types raikiri-traits" split).
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
mod tests {
    use super::*;
    use crate::dom::NodeId;
    use raikiri_style::property::CounterStyle;

    // ── CounterStack ─────────────────────────────────────

    mod counter_stack_tests {
        use super::*;

        #[test]
        fn fresh_stack_has_no_current_value() {
            let stack = CounterStack::default();
            assert_eq!(stack.current(), None);
            assert!(stack.values().is_empty());
        }

        #[test]
        fn reset_pushes_new_nested_scope_not_replace() {
            let mut stack = CounterStack::default();
            stack.reset(0);
            stack.reset(10);
            assert_eq!(stack.values(), &[0, 10]);
            assert_eq!(stack.current(), Some(10));
        }

        #[test]
        fn increment_on_empty_stack_implicitly_creates_at_delta() {
            let mut stack = CounterStack::default();
            stack.increment(3);
            assert_eq!(stack.current(), Some(3));
        }

        #[test]
        fn increment_mutates_only_innermost_frame() {
            let mut stack = CounterStack::default();
            stack.reset(0);
            stack.reset(10);
            stack.increment(5);
            assert_eq!(stack.values(), &[0, 15]);
        }

        #[test]
        fn increment_saturates_instead_of_panicking_or_wrapping_on_overflow() {
            // Regression check: `counter-reset: c 2147483647;
            // counter-increment: c 1` is spec-legal CSS. Must saturate at
            // i32::MAX, not panic (debug builds) or wrap to i32::MIN
            // (release builds).
            let mut stack = CounterStack::default();
            stack.reset(i32::MAX);
            stack.increment(1);
            assert_eq!(stack.current(), Some(i32::MAX));

            // Symmetric check on the negative side.
            let mut stack = CounterStack::default();
            stack.reset(i32::MIN);
            stack.increment(-1);
            assert_eq!(stack.current(), Some(i32::MIN));
        }

        #[test]
        fn set_on_empty_stack_implicitly_creates_at_value() {
            let mut stack = CounterStack::default();
            stack.set(7);
            assert_eq!(stack.current(), Some(7));
        }

        #[test]
        fn set_mutates_only_innermost_frame() {
            let mut stack = CounterStack::default();
            stack.reset(0);
            stack.reset(10);
            stack.set(99);
            assert_eq!(stack.values(), &[0, 99]);
        }

        #[test]
        fn pop_scope_removes_only_innermost_frame_and_reveals_previous() {
            let mut stack = CounterStack::default();
            stack.reset(1);
            stack.reset(2);
            assert_eq!(stack.pop_scope(), Some(2));
            assert_eq!(stack.current(), Some(1));
            assert_eq!(stack.pop_scope(), Some(1));
            assert_eq!(stack.current(), None);
        }

        #[test]
        fn pop_scope_on_empty_stack_returns_none_without_panicking() {
            let mut stack = CounterStack::default();
            assert_eq!(stack.pop_scope(), None);
        }
    }

    // ── NamedStringState ─────────────────────────────────

    mod named_string_state_tests {
        use super::*;

        #[test]
        fn fresh_state_has_no_snapshots() {
            let state = NamedStringState::default();
            assert_eq!(state.on_page_start(), None);
            assert_eq!(state.on_page_first_use(), None);
            assert_eq!(state.on_page_last_use(), None);
            assert_eq!(state.running(), None);
        }

        #[test]
        fn first_string_set_sets_first_last_and_running_but_not_start() {
            let mut state = NamedStringState::default();
            state.apply_string_set("A".to_owned());
            assert_eq!(state.on_page_first_use(), Some("A"));
            assert_eq!(state.on_page_last_use(), Some("A"));
            assert_eq!(state.running(), Some("A"));
            assert_eq!(state.on_page_start(), None);
        }

        #[test]
        fn second_string_set_same_page_updates_last_not_first() {
            let mut state = NamedStringState::default();
            state.apply_string_set("A".to_owned());
            state.apply_string_set("B".to_owned());
            assert_eq!(
                state.on_page_first_use(),
                Some("A"),
                "first-use must stay pinned to the first assignment on the page"
            );
            assert_eq!(state.on_page_last_use(), Some("B"));
            assert_eq!(state.running(), Some("B"));
        }

        #[test]
        fn begin_page_snapshots_running_into_start_and_clears_first_last() {
            let mut state = NamedStringState::default();
            state.apply_string_set("A".to_owned());
            state.begin_page();
            assert_eq!(state.on_page_start(), Some("A"));
            assert_eq!(state.on_page_first_use(), None);
            assert_eq!(state.on_page_last_use(), None);
            assert_eq!(state.running(), Some("A"));
        }

        #[test]
        fn multi_page_sequence_tracks_snapshot_timing_and_ordering() {
            let mut state = NamedStringState::default();
            state.apply_string_set("A".to_owned());

            state.begin_page();
            assert_eq!(state.on_page_start(), Some("A"));

            state.apply_string_set("B".to_owned());
            state.apply_string_set("C".to_owned());
            assert_eq!(state.on_page_first_use(), Some("B"));
            assert_eq!(state.on_page_last_use(), Some("C"));

            state.begin_page();
            assert_eq!(state.on_page_start(), Some("C"));
            assert_eq!(state.on_page_first_use(), None);
        }
    }

    // ── resolve_content_source ────────────────────────────

    mod resolve_content_source_tests {
        use super::*;

        #[test]
        fn literal_only_resolves_verbatim() {
            let source = ContentSource::new(vec![ContentValueItem::Literal("Ch. ".into())]);
            let counters = HashMap::new();
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("Ch. ".to_owned())
            );
        }

        #[test]
        fn counter_item_formats_current_value() {
            let mut counters = HashMap::new();
            let mut stack = CounterStack::default();
            stack.reset(3);
            counters.insert(Symbol::new("chapter"), stack);
            let source = ContentSource::new(vec![ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            }]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("3".to_owned())
            );
        }

        #[test]
        fn counter_item_absent_counter_formats_zero() {
            let counters = HashMap::new();
            let source = ContentSource::new(vec![ContentValueItem::Counter {
                name: Symbol::new("missing"),
                style: CounterStyle::Decimal,
            }]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("0".to_owned())
            );
        }

        #[test]
        fn counter_item_named_style_reaches_format_counter() {
            // Regression check for the format_counter/join_counter_stack
            // pub(crate) visibility widening: CounterStyle::Decimal alone
            // would exercise only
            // format_decimal, never touching the widened-visibility path. A
            // Named style forces resolve_content_source through
            // format_named_counter, proving the widening actually pays off
            // (not just compiles).
            let mut counters = HashMap::new();
            let mut stack = CounterStack::default();
            stack.reset(3);
            counters.insert(Symbol::new("chapter"), stack);
            let source = ContentSource::new(vec![ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::Named("upper-roman".into()),
            }]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("III".to_owned())
            );
        }

        #[test]
        fn counters_item_joins_full_stack_with_separator() {
            let mut counters = HashMap::new();
            let mut stack = CounterStack::default();
            stack.reset(1);
            stack.reset(1);
            counters.insert(Symbol::new("sec"), stack);
            let source = ContentSource::new(vec![ContentValueItem::Counters {
                name: Symbol::new("sec"),
                separator: ".".into(),
                style: CounterStyle::Decimal,
            }]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("1.1".to_owned())
            );
        }

        #[test]
        fn mixed_literal_and_counter_items_concatenate() {
            let mut counters = HashMap::new();
            let mut stack = CounterStack::default();
            stack.reset(2);
            counters.insert(Symbol::new("chapter"), stack);
            let source = ContentSource::new(vec![
                ContentValueItem::Literal("Ch. ".into()),
                ContentValueItem::Counter {
                    name: Symbol::new("chapter"),
                    style: CounterStyle::Decimal,
                },
            ]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                Some("Ch. 2".to_owned())
            );
        }

        #[test]
        fn unresolvable_variant_yields_none_not_fabricated_empty_string() {
            // See resolve_content_source's doc "Skipping, not fabricating":
            // Some("") and None are NOT interchangeable under CSS GCPM 3
            // §1.1.2's string() keyword rules, so an unresolvable item must
            // skip the whole assignment (None), never contribute "".
            let counters = HashMap::new();
            let source = ContentSource::new(vec![
                ContentValueItem::Literal("[".into()),
                ContentValueItem::Attr {
                    name: Symbol::new("data-title"),
                },
                ContentValueItem::Literal("]".into()),
            ]);
            assert_eq!(
                resolve_content_source(&source, &counters),
                None,
                "Attr needs DOM element access unavailable here — the whole \
                 content-list must be skipped, not partially resolved to \"[]\""
            );
        }
    }

    // ── PageContext::apply_directive ──────────────────────

    mod apply_directive_tests {
        use super::*;

        #[test]
        fn counter_directives_dispatch_to_named_stack() {
            let mut ctx = PageContext::default();
            let foo = Symbol::new("foo");
            let bar = Symbol::new("bar");
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: foo.clone(),
                value: 0,
            });
            ctx.apply_directive(&GcpmDirective::CounterIncrement {
                name: foo.clone(),
                delta: 3,
            });
            ctx.apply_directive(&GcpmDirective::CounterSet {
                name: bar.clone(),
                value: 9,
            });

            assert_eq!(ctx.counter(&foo).and_then(CounterStack::current), Some(3));
            assert_eq!(ctx.counter(&bar).and_then(CounterStack::current), Some(9));
        }

        #[test]
        fn same_element_reset_increment_set_order_matches_e81n_push_order() {
            let mut ctx = PageContext::default();
            let c = Symbol::new("c");
            let directives = vec![
                GcpmDirective::CounterReset {
                    name: c.clone(),
                    value: 0,
                },
                GcpmDirective::CounterIncrement {
                    name: c.clone(),
                    delta: 1,
                },
                GcpmDirective::CounterSet {
                    name: c.clone(),
                    value: 5,
                },
            ];
            for d in &directives {
                ctx.apply_directive(d);
            }
            assert_eq!(ctx.counter(&c).and_then(CounterStack::current), Some(5));
        }

        #[test]
        fn register_running_inserts_binding() {
            let mut ctx = PageContext::default();
            let name = Symbol::new("header");
            let id = RunningTemplateId::new(NodeId::new(7));
            ctx.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id,
            });
            assert_eq!(ctx.running_binding(&name), Some(id));
        }

        #[test]
        fn register_running_rebind_overwrites_previous_binding() {
            let mut ctx = PageContext::default();
            let name = Symbol::new("header");
            let id1 = RunningTemplateId::new(NodeId::new(7));
            let id2 = RunningTemplateId::new(NodeId::new(42));
            ctx.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id1,
            });
            ctx.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id2,
            });
            assert_eq!(
                ctx.running_binding(&name),
                Some(id2),
                "a later RegisterRunning for the same name must rebind, not be ignored"
            );
        }

        #[test]
        fn string_set_resolves_against_counter_values_at_apply_time() {
            let mut ctx = PageContext::default();
            let chapter = Symbol::new("chapter");
            let title = Symbol::new("title");

            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: chapter.clone(),
                value: 3,
            });
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: title.clone(),
                source: ContentSource::new(vec![
                    ContentValueItem::Literal("Ch. ".into()),
                    ContentValueItem::Counter {
                        name: chapter.clone(),
                        style: CounterStyle::Decimal,
                    },
                ]),
            });

            // Mutate the counter AFTER the string-set fired.
            ctx.apply_directive(&GcpmDirective::CounterIncrement {
                name: chapter.clone(),
                delta: 100,
            });

            assert_eq!(
                ctx.string_state(&title).and_then(NamedStringState::running),
                Some("Ch. 3"),
                "resolved text must reflect the value AT string-set time (3), \
                 not the later mutation (103)"
            );
            assert_eq!(
                ctx.counter(&chapter).and_then(CounterStack::current),
                Some(103)
            );
        }

        #[test]
        fn string_set_with_unresolvable_item_leaves_no_tracked_state() {
            // Regression check for the "skip, don't fabricate Some(\"\")"
            // fix — see resolve_content_source's doc "Skipping, not
            // fabricating" for why Some("") would be observably wrong
            // (CSS GCPM 3 §1.1.2 string() keyword fallback rules).
            let mut ctx = PageContext::default();
            let title = Symbol::new("title");
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: title.clone(),
                source: ContentSource::new(vec![ContentValueItem::Attr {
                    name: Symbol::new("data-title"),
                }]),
            });
            assert_eq!(
                ctx.string_state(&title),
                None,
                "an unresolvable string-set must not create a tracked entry at all"
            );
        }

        #[test]
        fn string_set_with_unresolvable_item_does_not_clobber_prior_resolvable_assignment() {
            let mut ctx = PageContext::default();
            let title = Symbol::new("title");
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: title.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("Ch. 1".into())]),
            });
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: title.clone(),
                source: ContentSource::new(vec![ContentValueItem::Attr {
                    name: Symbol::new("data-title"),
                }]),
            });
            assert_eq!(
                ctx.string_state(&title).and_then(NamedStringState::running),
                Some("Ch. 1"),
                "a later unresolvable string-set must skip entirely, not overwrite \
                 the prior resolvable assignment with fabricated empty text"
            );
        }

        #[test]
        fn begin_page_forwards_to_every_tracked_named_string() {
            let mut ctx = PageContext::default();
            let a = Symbol::new("a");
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: a.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("A".into())]),
            });

            ctx.begin_page(1);

            assert!(ctx.string_state(&a).unwrap().on_page_start().is_some());
            assert!(ctx.string_state(&a).unwrap().on_page_first_use().is_none());
        }

        #[test]
        fn register_target_registers_live_counter_snapshot() {
            let mut ctx = PageContext::default();
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: Symbol::new("chapter"),
                value: 1,
            });
            ctx.apply_directive(&GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("intro"),
            });

            let mut registry = ctx.targets().clone();
            let out = registry.resolve_target_counter(
                "#intro",
                Symbol::new("chapter"),
                CounterStyle::Decimal,
            );
            assert_eq!(out, crate::page::ResolveOutcome::Resolved("1".to_owned()));
        }

        #[test]
        fn register_target_unlike_pre_promotion_dom_local_no_op_really_mutates_targets() {
            // Regression check against the pre-promotion dom-local behavior
            // (the deleted raikiri_dom::gcpm::PhaseBWalkState treated
            // RegisterTarget as a documented no-op) — this promoted
            // apply_directive must NOT preserve that no-op; it owns
            // TargetRegistry for real now.
            let before = PageContext::default();
            let mut after = before.clone();
            after.apply_directive(&GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("intro"),
            });

            let mut before_registry = before.targets().clone();
            let mut after_registry = after.targets().clone();
            let before_out = before_registry
                .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
            let after_out = after_registry
                .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
            assert_ne!(
                before_out, after_out,
                "RegisterTarget must observably register the fragment (Pending → Resolved), \
                 unlike the dom-local no-op"
            );
        }

        #[test]
        fn register_target_does_not_clobber_a_richer_set_targets_registration() {
            // First-wins semantics (TargetRegistry::register) should mean a
            // fuller registration via set_targets survives a later
            // counts-only apply_directive RegisterTarget for the same
            // fragment_id — see apply_directive's RegisterTarget doc.
            let mut registry = TargetRegistry::new();
            let mut rich_info = TargetInfo::new();
            rich_info.text_parts.push((
                raikiri_style::property::ContentPart::Content,
                "Real Title".to_owned(),
            ));
            registry.register(Symbol::new("intro"), rich_info);

            let mut ctx = PageContext::default();
            ctx.set_targets(registry);
            ctx.apply_directive(&GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("intro"),
            });

            let mut result_registry = ctx.targets().clone();
            let out = result_registry
                .resolve_target_text("#intro", raikiri_style::property::ContentPart::Content);
            assert_eq!(
                out,
                crate::page::ResolveOutcome::Resolved("Real Title".to_owned()),
                "set_targets's richer TargetInfo must survive the later counts-only RegisterTarget"
            );
        }
    }

    // ── PageContext::pop_counter_scope ──────────────────────

    mod pop_counter_scope_tests {
        use super::*;

        #[test]
        fn pops_a_scope_pushed_via_counter_reset_directive() {
            let mut ctx = PageContext::default();
            let c = Symbol::new("c");
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: c.clone(),
                value: 0,
            });
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: c.clone(),
                value: 10,
            });
            ctx.apply_directive(&GcpmDirective::CounterIncrement {
                name: c.clone(),
                delta: 5,
            });
            assert_eq!(ctx.counter(&c).and_then(CounterStack::current), Some(15));

            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                ctx.pop_counter_scope(&c),
                Some(15),
                "must return the innermost (just-popped) scope's value"
            );
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                ctx.counter(&c).and_then(CounterStack::current),
                Some(0),
                "after popping the inner scope, a subsequent read must fall back to \
                 the outer scope pushed by the first CounterReset"
            );
        }

        #[test]
        fn popping_the_only_scope_falls_back_to_absent() {
            let mut ctx = PageContext::default();
            let c = Symbol::new("c");
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: c.clone(),
                value: 3,
            });
            assert_eq!(ctx.pop_counter_scope(&c), Some(3));
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                ctx.counter(&c).and_then(CounterStack::current),
                None,
                "popping the only tracked scope must leave the counter reading as \
                 absent, not silently resurrect a value"
            );
        }

        #[test]
        fn popping_a_name_with_no_tracked_counter_returns_none() {
            let mut ctx = PageContext::default();
            let untouched = Symbol::new("never-reset");
            assert_eq!(ctx.pop_counter_scope(&untouched), None);
        }
    }

    // ── PageContext::begin_page ────────────────────────────

    mod begin_page_tests {
        use super::*;

        #[test]
        fn begin_page_updates_the_page_index_field() {
            let mut ctx = PageContext::default();
            assert_eq!(ctx.page_index, 0);
            ctx.begin_page(3);
            assert_eq!(ctx.page_index, 3);
        }

        #[test]
        fn begin_page_forwards_page_index_to_the_targets_registry() {
            // The coordination this method exists for: a slot minted after
            // begin_page(7) must be stamped with page_index 7, not the
            // registry's stale prior value — that staleness is exactly the
            // desync a bare `ctx.page_index = 7` write (with no registry
            // call) would leave behind.
            let mut ctx = PageContext::default();
            ctx.begin_page(7);

            let mut registry = ctx.targets().clone();
            let out = registry.resolve_target_counter(
                "#not-yet-registered",
                Symbol::new("chapter"),
                CounterStyle::Decimal,
            );
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                out,
                crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                    page_index: 7,
                    sequence: 0,
                }),
                "a slot minted after begin_page(7) must be stamped with page_index 7"
            );
        }

        #[test]
        fn begin_page_resets_the_registrys_page_local_sequence_counter() {
            let mut ctx = PageContext::default();

            // Mint a slot on page 0 to advance next_sequence past 0.
            let mut registry = ctx.targets().clone();
            registry.resolve_target_counter("#a", Symbol::new("c"), CounterStyle::Decimal);
            ctx.set_targets(registry);

            ctx.begin_page(1);

            let mut registry = ctx.targets().clone();
            let out =
                registry.resolve_target_counter("#b", Symbol::new("c"), CounterStyle::Decimal);
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                out,
                crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                    page_index: 1,
                    sequence: 0,
                }),
                "the page-local sequence must restart at 0 on a new page, not \
                 continue from page 0's count"
            );
        }

        #[test]
        #[should_panic(expected = "monotonically")]
        fn begin_page_backward_call_panics_through_page_context() {
            // TargetRegistry::begin_page's own monotonic-page_index assert
            // must be reachable through this coordinating method, not just
            // when TargetRegistry is driven directly.
            let mut ctx = PageContext::default();
            ctx.begin_page(5);
            ctx.begin_page(3);
        }

        #[test]
        fn begin_page_backward_call_panic_leaves_page_index_unchanged() {
            // Registry-first call order (see begin_page's doc "Call order
            // inside this method is registry-first"): the assert! in
            // TargetRegistry::begin_page fires before self.page_index is
            // reassigned, so a caught panic must leave page_index paired
            // with the *last successful* call, not torn (advanced) while
            // targets stayed on the old page.
            let mut ctx = PageContext::default();
            ctx.begin_page(5);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ctx.begin_page(3);
            }));
            assert!(result.is_err(), "backward begin_page call must panic");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                ctx.page_index, 5,
                "a panicking begin_page call must not partially apply — page_index \
                 must remain paired with the last successful call"
            );
        }
    }

    // ── PageContext::set_targets ──────────────────────────

    mod set_targets_tests {
        use super::*;

        #[test]
        fn set_targets_replaces_the_registry_wholesale() {
            let mut registry = TargetRegistry::new();
            registry.register(Symbol::new("ch1"), TargetInfo::new());

            let mut ctx = PageContext::default();
            ctx.set_targets(registry);

            let mut result = ctx.targets().clone();
            let out = result.resolve_target_counter(
                "#ch1",
                Symbol::new("nonexistent"),
                CounterStyle::Decimal,
            );
            // Missing counter on an existing (registered) target resolves
            // immediately to "0", proving #ch1 landed via set_targets.
            assert_eq!(out, crate::page::ResolveOutcome::Resolved("0".to_owned()));
        }

        #[test]
        fn set_targets_after_register_target_wipes_the_earlier_registration() {
            // Regression check: the apply_directive RegisterTarget arm's doc
            // documents that
            // set_targets called AFTER a prior RegisterTarget directive is a
            // wholesale wipe, not an order-independent merge — only the
            // reverse ordering (set_targets first) had a test. This pins the
            // other direction: a fragment registered via apply_directive
            // must NOT survive a later set_targets call whose registry
            // doesn't contain it.
            let mut ctx = PageContext::default();
            ctx.apply_directive(&GcpmDirective::CounterReset {
                name: Symbol::new("chapter"),
                value: 1,
            });
            ctx.apply_directive(&GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("intro"),
            });

            // A fresh, unrelated registry that never saw "intro".
            ctx.set_targets(TargetRegistry::new());

            let mut result = ctx.targets().clone();
            let out = result.resolve_target_counter(
                "#intro",
                Symbol::new("chapter"),
                CounterStyle::Decimal,
            );
            assert_eq!(
                out,
                crate::page::ResolveOutcome::Pending(crate::error::TargetSlotId {
                    page_index: 0,
                    sequence: 0,
                }),
                "set_targets after RegisterTarget must wipe the earlier registration, \
                 not merge with or preserve it"
            );
        }
    }

    // ── page_index / page_name plain fields ───────────────

    mod plain_field_tests {
        use super::*;

        #[test]
        fn page_index_defaults_to_zero_and_is_directly_settable() {
            let mut ctx = PageContext::default();
            assert_eq!(ctx.page_index, 0);
            ctx.page_index = 5;
            assert_eq!(ctx.page_index, 5);
        }

        #[test]
        fn page_name_defaults_to_none_and_is_directly_settable() {
            let mut ctx = PageContext::default();
            assert_eq!(ctx.page_name, None);
            ctx.page_name = Some(Symbol::new("chapter-front"));
            assert_eq!(ctx.page_name, Some(Symbol::new("chapter-front")));
        }
    }

    // ── promotion parity ──────────────────────────────────

    #[test]
    fn page_context_new_and_default_agree_with_umbrella_contract() {
        // crates/raikiri/tests/external_consumer.rs and strategy.rs's
        // TargetResolver already depend on both constructors continuing to
        // work across this promotion.
        let a = PageContext::new();
        let b = PageContext::default();
        assert_eq!(a.page_index, b.page_index);
        assert_eq!(a.page_name, b.page_name);
    }
}
