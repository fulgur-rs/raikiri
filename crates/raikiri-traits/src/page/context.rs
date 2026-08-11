//! `PageContext` — GCPM Phase B runtime state (design §7.0/§7.1/§7.2).
//!
//! **Promotion history.** Design §7.2
//! (`docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md` lines
//! 1940-1965) gives `PageContext`'s canonical 6-field shape. Until bd
//! raikiri-spike-8ejw.1, this was an empty `#[non_exhaustive]` placeholder
//! (M1.1) while the walk algorithm (nested counter scopes, named-string
//! 4-snapshot timing, running-binding rebind) was built and unit tested
//! dom-locally at `raikiri_dom::gcpm::PhaseBWalkState` (bd raikiri-spike-8ejw)
//! — see that module's doc for why: populating this struct's fields requires
//! new raikiri-traits `pub` accessor/mutator methods, a genuine `wall/traits`
//! crossing. This module is that promotion, landed per the human decision
//! recorded on bd raikiri-spike-8ejw.1 (2026-08-11 comment):
//!
//! 1. The 4 `HashMap`-shaped fields (`counters` / `strings` / `running` /
//!    `targets`) are encapsulated behind the single
//!    [`PageContext::apply_directive`] entry point — no per-field
//!    accessor/mutator methods for these 4 (read accessors are narrower and
//!    separate, see below).
//! 2. `page_index` / `page_name` are plain `pub` fields — copy-cheap
//!    scalars with no invariant to protect.
//!
//! **Type-promotion design decision** (bd raikiri-spike-8ejw.1 step 2): the
//! dom-local `raikiri_dom::gcpm::CounterStack` /
//! `raikiri_dom::gcpm::NamedStringState` *shapes* are promoted
//! (moved) here, matching the `super::target` precedent (`TargetRegistry`
//! / `TargetInfo` canonical impl lives in raikiri-traits, raikiri-dom only
//! hosts producers). `raikiri_dom::gcpm`'s own dom-local mirror types are
//! **deliberately left in place**, not deleted: no production driver in
//! raikiri-dom calls `PageContext::apply_directive` yet (wiring a real
//! DOM-tree-walking driver — `CounterStack::pop_scope` / `NamedStringState`
//! page-boundary call sites — is bd raikiri-spike-si32, explicitly out of
//! this task's scope) — `raikiri_dom::gcpm::PhaseBWalkState` remains the
//! *only* currently-exercised implementation reachable from raikiri-dom's
//! actual code path (`apply_running_template_directives`). Deleting it here
//! would strand that call site with nothing to call until si32 lands.
//! `raikiri_dom::gcpm`'s module doc is updated to point here instead of
//! carrying a stale "not yet landed" note (bd raikiri-spike-8ejw.1 step 8).
//!
//! **Field-shape divergence from the dom-local mirror, now resolved**: the
//! dom-local `NamedStringState` stores `Option<StringSnapshot>` (a
//! `ContentSource` + a raw counter-values snapshot) instead of design §7.2's
//! canonical `Option<String>`, because `raikiri_dom::gcpm` could not reach
//! `super::target`'s private `format_counter` / `join_counter_stack`
//! helpers (a documented gap, flagged on bd raikiri-spike-8ejw.1's
//! 2026-08-11 04:00 comment). Now that this promotion lives *inside*
//! raikiri-traits, that gap dissolves for real: [`NamedStringState`] here
//! matches the design doc's `Option<String>` shape exactly, and
//! [`resolve_content_source`] does full eager resolution at
//! [`PageContext::apply_directive`] time using those two helpers (widened
//! from private to `pub(crate)` — see that fn's doc for why `pub(crate)`
//! rather than full `pub`: [`PageContext`] and `super::target` are sibling
//! `page` submodules, so a same-crate, narrower-than-`pub` visibility
//! already suffices; there is no cross-crate need).
//!
//! **`RegisterTarget` now really mutates `self.targets`** — unlike
//! `raikiri_dom::gcpm`'s documented no-op (that module doesn't own
//! `TargetRegistry` at all). See [`PageContext::apply_directive`]'s doc for
//! what it can and cannot populate from a bare directive, and
//! [`PageContext::set_targets`] for the complementary bulk-wiring path bd
//! raikiri-spike-0nyv's register-site walker uses.

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
/// Promoted from `raikiri_dom::gcpm::CounterStack` (bd raikiri-spike-8ejw.1)
/// — same algorithm, now `pub` (design §7.2 gives `PageContext.counters:
/// HashMap<Symbol, CounterStack>` with no `pub` on the field itself, so the
/// *value type* returned by [`PageContext::counter`] must be nameable from
/// outside this crate).
///
/// **Caller invariant — scope exit is the caller's responsibility.**
/// [`Self::reset`] unconditionally *pushes* a new frame; it never replaces
/// the innermost one. Popping that frame when the walk leaves the
/// originating element's subtree is **not** done automatically here —
/// [`Self::pop_scope`] exists for a future DOM-tree-driven walker to call at
/// the right point (subtree exit, matching the real CSS scoping rule where
/// two *sibling* elements each resetting the same counter get independent,
/// same-depth scopes, not accumulating nesting) — bd raikiri-spike-si32, not
/// yet landed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    pub fn increment(&mut self, delta: i32) {
        match self.frames.last_mut() {
            Some(top) => *top += delta,
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
    /// — no driver currently calls this (bd raikiri-spike-si32, not yet
    /// landed); exposed for that future DOM-tree-driven walker.
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
/// Promoted from `raikiri_dom::gcpm::NamedStringState` (bd
/// raikiri-spike-8ejw.1) — same 4-snapshot timing rules, but with the
/// design-doc-canonical `Option<String>` field type (fully resolved text)
/// instead of that module's dom-local `Option<StringSnapshot>` deferral; see
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
/// over unchanged from the dom-local mirror (`raikiri_dom::gcpm`) — the
/// consuming `string()` implementation itself is a future consumer's task,
/// not this promotion's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// No production driver calls this yet — the per-page walk that would
    /// call it is bd raikiri-spike-si32, not yet landed.
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
/// **Only [`ContentValueItem::Literal`] / `Counter` / `Counters`
/// resolve to real text here.** Every other variant
/// (`String`/`Element`/`Content`/`Attr`/`TargetCounter`/`TargetCounters`/
/// `TargetText`/`Image`/`Contents`/`Quote`/`Leader`) needs context this
/// directive-only resolution path structurally cannot have: DOM element
/// access (`String`/`Element`/`Content`/`Attr`), mutable [`TargetRegistry`]
/// access for a second resolve pass (`TargetCounter`/`TargetCounters`/
/// `TargetText` — and [`PageContext::apply_directive`] already holds
/// `&mut self.targets` exclusively for the *current* directive, so
/// re-entrant resolution isn't available either), or paint-time layout
/// machinery (`Image`/`Contents`/`Quote`/`Leader`). Contributing nothing for
/// those is a fail-closed narrowing (原則3), not a stub bug — same
/// discipline `raikiri_dom::target`'s "`ContentPart::Content` only" text-part
/// narrowing already established, and matches the dom-local mirror's
/// documented rationale for deferring resolution entirely rather than
/// guessing at wrong text.
fn resolve_content_source(
    source: &ContentSource,
    counters: &HashMap<Symbol, CounterStack>,
) -> String {
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
                // Fail-closed narrowing — see this fn's doc.
            }
        }
    }
    out
}

// ── PageContext ──────────────────────────────────────────────────────────

/// GCPM Phase B runtime state — the mutable page-local state a Phase B walk
/// (design §7.0 "runtime side") builds up as it applies [`GcpmDirective`]s
/// and reads back for `content` resolution.
///
/// See module-level doc for the promotion history and the human decision
/// (bd raikiri-spike-8ejw.1) this shape implements:
///
/// - `counters` / `strings` / `running` / `targets` are private, mutated
///   *only* via [`Self::apply_directive`] (single entry point — no per-field
///   accessor/mutator), read via the narrow accessors below.
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
    /// Plain `pub` field — copy-cheap scalar, no invariant to protect (human
    /// decision, bd raikiri-spike-8ejw.1 2026-08-11 comment).
    pub page_index: u32,
    /// `@page` named-page association in effect for this page, if any
    /// (design §7.2). Plain `pub` field — same rationale as `page_index`.
    pub page_name: Option<Symbol>,
}

impl PageContext {
    /// Construct an empty `PageContext` (stable across the M1.1 → M4
    /// promotion — `crates/raikiri/tests/external_consumer.rs` and
    /// `crates/raikiri-traits/src/strategy.rs`'s `TargetResolver` already
    /// depend on this constructor and `Default` continuing to work).
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one [`GcpmDirective`] to this context — the single entry point
    /// for mutating `counters` / `strings` / `running` / `targets` (human
    /// decision, bd raikiri-spike-8ejw.1). Mirrors
    /// `raikiri_dom::gcpm::PhaseBWalkState::apply_directive`'s dispatch, with
    /// two differences documented on the relevant arms below:
    /// `StringSet` fully resolves text (module doc "Field-shape
    /// divergence") and `RegisterTarget` really registers (module doc
    /// "`RegisterTarget` now really mutates `self.targets`").
    ///
    /// **Exhaustive same-crate match, no wildcard arm.** [`GcpmDirective`] is
    /// defined in this crate; `#[non_exhaustive]`'s "downstream `match` needs
    /// a wildcard" restriction is a cross-crate rule, so unlike
    /// `raikiri_dom::gcpm`'s necessarily-wildcarded copy, this match lists
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
                let resolved = resolve_content_source(source, &self.counters);
                self.strings
                    .entry(name.clone())
                    .or_default()
                    .apply_string_set(resolved);
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
                // StringSet already uses) — no text_parts, since DOM element
                // access isn't available at this call site (same fail-closed
                // narrowing as resolve_content_source). The richer,
                // text-carrying registration path is `Self::set_targets`,
                // fed by `raikiri_dom::target::build_target_registry`'s
                // dedicated DOM walk — see that method's doc.
                //
                // TargetRegistry::register is first-wins (entry().or_insert)
                // by design, so if set_targets already populated this
                // fragment_id with a fuller TargetInfo (text included), this
                // arm's counts-only registration is naturally a no-op for
                // that fragment — no clobbering, regardless of call order.
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

    /// Page-boundary hook — forwards to every tracked [`NamedStringState`].
    /// See [`NamedStringState::begin_page`] for why this has no production
    /// caller yet (bd raikiri-spike-si32).
    pub fn begin_page(&mut self) {
        for state in self.strings.values_mut() {
            state.begin_page();
        }
    }

    /// Replace `targets` wholesale with a pre-built [`TargetRegistry`] — the
    /// bulk-wiring path for bd raikiri-spike-0nyv's
    /// `raikiri_dom::target::build_target_registry` register-site walker,
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
            assert_eq!(resolve_content_source(&source, &counters), "Ch. ");
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
            assert_eq!(resolve_content_source(&source, &counters), "3");
        }

        #[test]
        fn counter_item_absent_counter_formats_zero() {
            let counters = HashMap::new();
            let source = ContentSource::new(vec![ContentValueItem::Counter {
                name: Symbol::new("missing"),
                style: CounterStyle::Decimal,
            }]);
            assert_eq!(resolve_content_source(&source, &counters), "0");
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
            assert_eq!(resolve_content_source(&source, &counters), "1.1");
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
            assert_eq!(resolve_content_source(&source, &counters), "Ch. 2");
        }

        #[test]
        fn unresolvable_variant_contributes_empty_string() {
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
                "[]",
                "Attr needs DOM element access unavailable here — contributes nothing, \
                 not a placeholder or panic"
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
        fn begin_page_forwards_to_every_tracked_named_string() {
            let mut ctx = PageContext::default();
            let a = Symbol::new("a");
            ctx.apply_directive(&GcpmDirective::StringSet {
                name: a.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("A".into())]),
            });

            ctx.begin_page();

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
        fn register_target_unlike_dom_local_gcpm_no_op_really_mutates_targets() {
            // Regression pin against the pre-promotion dom-local behavior
            // (raikiri_dom::gcpm::PhaseBWalkState treats RegisterTarget as a
            // documented no-op) — this promoted apply_directive must NOT
            // preserve that no-op; it owns TargetRegistry for real now.
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
