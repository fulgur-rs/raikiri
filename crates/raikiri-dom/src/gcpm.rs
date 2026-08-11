//! GCPM Phase B directive-application walk — dom-local state (design
//! §7.0/§7.1/§7.2).
//!
//! Design doc §7.0 (`docs/superpowers/specs/2026-07-13-raikiri-rebuild-design.md`
//! lines 1898-1902) assigns raikiri-dom the "runtime side" of GCPM: owning
//! `PageContext` (counter tree, named-string 4-snapshot, running bindings)
//! and applying [`GcpmDirective`] entries during a walk ("Phase B walk").
//! §7.2 (lines 1940-1965) gives the canonical shape those 3 fields would
//! take on `raikiri_traits::PageContext` — `counters: HashMap<Symbol,
//! CounterStack>`, `strings: HashMap<Symbol, NamedStringState>`,
//! `running: HashMap<Symbol, RunningTemplateId>`.
//!
//! **Promotion landed** — bd raikiri-spike-8ejw.1 (human-reviewed
//! wall/traits crossing) added the `pub` accessor/mutator surface this
//! required and promoted this module's algorithm onto
//! `raikiri_traits::page::PageContext` (`crates/raikiri-traits/src/page/context.rs`):
//! `PageContext::apply_directive` is now the canonical, single-entry-point
//! implementation, with `CounterStack` / `NamedStringState` promoted
//! alongside it as `pub` raikiri-traits types. **This module is deliberately
//! retained, not deleted** — no production driver in raikiri-dom calls
//! `PageContext::apply_directive` yet; wiring a real DOM-tree-walking driver
//! (`CounterStack::pop_scope` / `NamedStringState` page-boundary call sites)
//! is bd raikiri-spike-si32, not yet landed. Until si32 rewires
//! [`apply_running_template_directives`] (this module's current caller) to
//! target `raikiri_traits::PageContext` directly, this dom-local mirror
//! remains the *only* currently-exercised implementation reachable from
//! raikiri-dom's actual code path.
//!
//! **This module builds the algorithm + state dom-locally**, entirely inside
//! raikiri-dom, so the walk's correctness (nested counter scopes, snapshot
//! timing, running-binding rebind) can be built and unit tested independent
//! of the raikiri-traits promotion (same discipline
//! [`crate::running`]'s `ParsedRunningTemplate::directives` field already
//! applied to *this* module before it existed — see that field's doc
//! comment).
//!
//! **Input source** — [`PhaseBWalkState::apply_directive`] consumes one
//! [`GcpmDirective`] at a time; [`apply_running_template_directives`] is the
//! current concrete caller, iterating
//! [`crate::running::ParsedRunningTemplate::directives`] front-to-back (bd
//! raikiri-spike-e81n's producer). That Vec is built by a flat preorder
//! subtree walk with no subtree-*exit* markers (see
//! `collect_running_template`'s doc) — see [`CounterStack`]'s type-level
//! "Caller invariant" note for what that does and doesn't let this walk
//! prove yet.
//!
//! **`RegisterTarget` is out of scope for *this* dom-local mirror** — bd
//! raikiri-spike-0nyv owns the `TargetRegistry` producer
//! (`crate::target::build_target_registry`); this module has no
//! `TargetRegistry` field to wire it into, so
//! [`PhaseBWalkState::apply_directive`] treats it as a documented no-op.
//! **Unlike this mirror, the promoted `raikiri_traits::PageContext::apply_directive`
//! really registers** (it owns a `targets: TargetRegistry` field) — see that
//! method's doc for exactly what it can and cannot populate from a bare
//! directive.

use std::collections::HashMap;

use raikiri_traits::{ContentSource, GcpmDirective, RunningTemplateId, Symbol};

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
/// **Caller invariant — scope exit is the caller's responsibility.**
/// [`Self::reset`] unconditionally *pushes* a new frame; it never replaces
/// the innermost one. Popping that frame when the walk leaves the
/// originating element's subtree is **not** done automatically here —
/// [`Self::pop_scope`] exists for a future DOM-tree-driven walker to call at
/// the right point (subtree exit), matching the real CSS scoping rule where
/// two *sibling* elements each resetting the same counter get independent,
/// same-depth scopes (not accumulating nesting). **No such driver exists
/// yet** — [`apply_running_template_directives`] (this module's current
/// caller) iterates a flat `Vec<GcpmDirective>` with no subtree-exit
/// markers (see module-level doc), so nothing currently calls `pop_scope`.
/// Applying two sibling `CounterReset`s through this walk today will
/// therefore accumulate depth instead of resetting at the same level. This
/// is a known, documented limitation of the *driver*, not a bug in the
/// push/pop primitives themselves — the unit tests below pin push/pop
/// correctness directly (the primitives a future driver will call), not the
/// "real DOM walk pops at the right points" integration, which is out of
/// this task's scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CounterStack {
    /// Nested scope frames, outermost first. Empty = the counter has not
    /// been instantiated yet (CSS Lists 3 §4.2: an un-instantiated counter
    /// behaves as absent until reset/increment/set creates one).
    frames: Vec<i32>,
}

impl CounterStack {
    /// `counter-reset: name value` (CSS Lists 3 §4.1) — push a new nested
    /// scope frame at `value`. See the type-level "Caller invariant" note:
    /// always pushes, never replaces the innermost frame.
    pub(crate) fn reset(&mut self, value: i32) {
        self.frames.push(value);
    }

    /// `counter-increment: name delta` (CSS Lists 3 §4.2
    /// <https://www.w3.org/TR/css-lists-3/#propdef-counter-increment>) — add
    /// `delta` to the innermost frame. An un-instantiated counter (empty
    /// stack) is implicitly created at `delta` — the spec's "increment a
    /// non-existent counter" path instantiates it at 0 and then applies the
    /// increment, which is equivalent to starting the new frame at `delta`
    /// directly.
    pub(crate) fn increment(&mut self, delta: i32) {
        match self.frames.last_mut() {
            Some(top) => *top += delta,
            None => self.frames.push(delta),
        }
    }

    /// `counter-set: name value` (CSS Lists 3 §4.2
    /// <https://www.w3.org/TR/css-lists-3/#propdef-counter-set>) — set the
    /// innermost frame to `value`. An un-instantiated counter is implicitly
    /// created at `value` (mirrors [`Self::increment`]'s implicit-create).
    pub(crate) fn set(&mut self, value: i32) {
        match self.frames.last_mut() {
            Some(top) => *top = value,
            None => self.frames.push(value),
        }
    }

    /// Exit the innermost nested scope, returning its value (`None` if the
    /// stack was already empty). See the type-level "Caller invariant" note
    /// — no directive currently triggers this; it's exposed for a future
    /// DOM-tree-driven walker.
    #[allow(
        dead_code,
        reason = "No driver calls this yet (see type-level 'Caller invariant' \
                  note) — exercised directly by this module's unit tests, \
                  exposed for a future DOM-tree-driven walker."
    )]
    pub(crate) fn pop_scope(&mut self) -> Option<i32> {
        self.frames.pop()
    }

    /// The innermost (currently effective) value, if any scope exists.
    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn current(&self) -> Option<i32> {
        self.frames.last().copied()
    }

    /// Every nested frame, outermost first — the shape `counters()`
    /// (CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>)
    /// joins with a separator when rendering. Consuming-directive/paint-time
    /// concern (not exercised by this walk), but the accessor a future
    /// consumer — and this module's own [`StringSnapshot::counter_snapshot`]
    /// freeze — needs.
    pub(crate) fn values(&self) -> &[i32] {
        &self.frames
    }
}

// ── NamedStringState ─────────────────────────────────────────────────────

/// A `string-set` value frozen at the moment its directive was applied: the
/// resolved-shape content-list plus every counter's nested-stack values at
/// that point in the walk.
///
/// **Why the counter freeze is required, not optional** — CSS GCPM 3 §1.1.1
/// <https://www.w3.org/TR/css-gcpm-3/#setting-named-strings-the-string-set-pro>:
/// "The content values of named strings are assigned at the point when the
/// content box of the element is first created (or would have been created
/// if the element's display value is none)." The spec fixes *when* the
/// assignment happens; it does not spell out how a `counter()`/`counters()`
/// reference inside the content-list resolves relative to that instant.
/// Taking the assignment-time rule at face value, such a reference must
/// resolve against the counter values in scope *at the element the
/// `string-set` fired on* — not against whatever the counters have drifted
/// to by the time some later consumer actually renders the stored value.
/// Deferring full *text* resolution is fine (see [`NamedStringState`]'s
/// type-level note for why), but deferring *which counter values apply*
/// would silently resolve against the wrong point in the walk, so
/// [`Self::counter_snapshot`] freezes the whole counters map eagerly at
/// [`PhaseBWalkState::apply_directive`] time.
///
/// Same "freeze at directive time" shape as
/// [`raikiri_traits::TargetInfo::counters`]
/// (`counters: HashMap<Symbol, Vec<i32>>`, doc'd there as "定義時点の counter
/// 値") and design doc line 2698's `counter_snapshot: HashMap<Symbol, i32>`
/// — this type is the `StringSet`-side counterpart of that established
/// precedent (this module freezes the *whole* nested stack per name, not
/// just the innermost value, since a `counters()` reference inside the
/// content-list needs every frame).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StringSnapshot {
    /// The resolved-shape content-list as carried by the directive.
    pub(crate) source: ContentSource,
    /// Every counter's full nested stack (see [`CounterStack::values`]),
    /// frozen at the moment this `string-set` fired.
    pub(crate) counter_snapshot: HashMap<Symbol, Vec<i32>>,
}

/// Dom-local mirror of design §7.2's `NamedStringState` — the `string-set`
/// 4-snapshot state machine (CSS GCPM 3 §1.1.1
/// <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set> / §1.1.2
/// <https://www.w3.org/TR/css-gcpm-3/#string-first>).
///
/// **Field-type divergence from design §7.2**: the design doc types every
/// snapshot field `Option<String>` (fully resolved text). This dom-local
/// mirror stores [`Option<StringSnapshot>`] instead — full text resolution
/// of a `string-set`'s content-list (in particular `counter()`/`counters()`
/// with a non-decimal [`raikiri_style::property::CounterStyle::Named`])
/// needed `raikiri_traits::page::target`'s private `format_counter` /
/// `join_counter_stack` helpers, both `wall/traits`-gated (unreachable from
/// raikiri-dom) at the time this module was written. Partially resolving
/// (Literal-only, or decimal-only counters) was considered and rejected: it
/// would silently bake wrong text for `Named` counter styles and for
/// `Attr`/`Content` items (which additionally need the originating DOM
/// element — also unavailable to this flat-directive walk, see module-level
/// doc), which is worse than deferring resolution entirely (原則3,
/// fail-closed).
///
/// **Resolved by the promotion** (bd raikiri-spike-8ejw.1): those two
/// helpers are now `pub(crate)` inside raikiri-traits, and the promoted
/// `raikiri_traits::page::context::NamedStringState` matches design §7.2
/// exactly (`Option<String>`, eagerly resolved at
/// `PageContext::apply_directive` time — see that promoted module's
/// `resolve_content_source` doc for the still-deferred non-counter variants,
/// same fail-closed rationale as this doc's previous paragraph). This
/// dom-local `StringSnapshot`/`Option<StringSnapshot>` deferral remains
/// correct *for this mirror specifically* — it's simply superseded, not
/// broken, by the promoted type.
///
/// **4-snapshot field mapping** — design §7.2's prose names the 4 snapshots
/// "start/first/last/first-except" (the CSS GCPM 3 §1.1.2 keyword set) but
/// the struct itself has `on_page_start` / `on_page_first_use` /
/// `on_page_last_use` / `running` (no `first_except` field). Following the
/// struct (the normative part) rather than the prose: `first-except` is
/// derivable at read time from `on_page_first_use` plus page-position
/// context (spec: same as `first`, except empty on the page where a
/// `string-set` assignment for this name actually occurs — i.e. empty
/// whenever `on_page_first_use` is `Some` for the current page, a
/// page-local condition, not an element-position one; see the keyword
/// table below for the precise rule) rather than stored as its own
/// snapshot slot — deliberate, not an oversight.
///
/// **None of the 4 keywords reduce to a single stored field read** — a
/// future `string()` implementation must combine these fields with
/// page-position context for 2 of the 4 keywords, not just `first-except`:
/// `first` → `on_page_first_use`, else `on_page_start` (CSS GCPM 3
/// §1.1.2 <https://www.w3.org/TR/css-gcpm-3/#string-first>: "the value of
/// the first assignment on the page ... If there is no assignment on the
/// page, the entry value is used"). `start` → **if the querying element is
/// the page's first formatted element**, `on_page_first_use`; **otherwise**
/// `on_page_start` (spec: "If the element is the first element on the
/// page, the value of the first assignment is used. Otherwise the entry
/// value is used") — this element-position condition is *not* captured by
/// this state machine at all and must come from the caller. `last` →
/// `on_page_last_use`, else `on_page_start` (exit value). `first-except` →
/// `on_page_first_use.is_some()` ? empty : `on_page_start` (condition is
/// "the page where the value is assigned", i.e. page-local, not
/// element-position).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NamedStringState {
    /// The value in effect at the start of the page — `running` as it stood
    /// when [`Self::begin_page`] was last called (CSS GCPM 3 §1.1.2's "entry
    /// value": the value in effect at the end of the previous page, carried
    /// forward via `running`). `None` before any `string-set` has ever fired
    /// in the document.
    on_page_start: Option<StringSnapshot>,
    /// The first `string-set` assignment applied since the current page
    /// began. `None` until the first assignment on this page.
    on_page_first_use: Option<StringSnapshot>,
    /// The most recent `string-set` assignment applied since the current
    /// page began. `None` until the first assignment on this page;
    /// thereafter tracks every subsequent assignment (including the first).
    on_page_last_use: Option<StringSnapshot>,
    /// The always-current value — every `string-set` application overwrites
    /// this immediately, independent of page boundaries.
    /// [`Self::begin_page`] reads this to seed the next page's
    /// `on_page_start`.
    running: Option<StringSnapshot>,
}

impl NamedStringState {
    /// Page-boundary hook — snapshot the current `running` value as the new
    /// page's entry/start value, and clear the per-page first/last-use
    /// trackers (a fresh page starts with no assignments of its own).
    ///
    /// No production driver calls this yet — no per-page walk exists (see
    /// module-level doc); exposed for a future PageStream-driven walker and
    /// exercised directly by this module's unit tests.
    #[allow(
        dead_code,
        reason = "No production driver calls this yet — no per-page walk \
                  exists (see module-level doc); exercised directly by this \
                  module's unit tests."
    )]
    pub(crate) fn begin_page(&mut self) {
        self.on_page_start = self.running.clone();
        self.on_page_first_use = None;
        self.on_page_last_use = None;
    }

    /// Apply a `string-set` directive's frozen [`StringSnapshot`]: always
    /// updates `running`; updates `on_page_first_use` only if this is the
    /// first assignment since the last [`Self::begin_page`] (or since
    /// construction, if `begin_page` has never run); always updates
    /// `on_page_last_use`.
    pub(crate) fn apply_string_set(&mut self, snapshot: StringSnapshot) {
        if self.on_page_first_use.is_none() {
            self.on_page_first_use = Some(snapshot.clone());
        }
        self.on_page_last_use = Some(snapshot.clone());
        self.running = Some(snapshot);
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn on_page_start(&self) -> Option<&StringSnapshot> {
        self.on_page_start.as_ref()
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn on_page_first_use(&self) -> Option<&StringSnapshot> {
        self.on_page_first_use.as_ref()
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn on_page_last_use(&self) -> Option<&StringSnapshot> {
        self.on_page_last_use.as_ref()
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn running(&self) -> Option<&StringSnapshot> {
        self.running.as_ref()
    }
}

// ── PhaseBWalkState ──────────────────────────────────────────────────────

/// Dom-local Phase B GCPM directive-application walk state (design
/// §7.0/§7.1/§7.2 "Phase B walk").
///
/// **Not `raikiri_traits::PageContext`.** This type is the dom-internal
/// mirror of 3 of `PageContext`'s 6 §7.2 fields (`counters` / `strings` /
/// `running` — `targets` is bd raikiri-spike-0nyv's `TargetRegistry`
/// territory, `page_index` / `page_name` are per-page driver state neither
/// task owns): the algorithm this module was scoped to build and unit test
/// in isolation. **Promotion onto `PageContext` has landed** — bd
/// raikiri-spike-8ejw.1, `raikiri_traits::page::context::PageContext`. This
/// type is kept as the pre-promotion working area (module-level doc
/// "Promotion landed" explains why it isn't deleted yet), *not* an
/// unfinished duplicate — its `RegisterTarget` no-op and lack of a
/// `TargetRegistry` field are permanent characteristics of this dom-local
/// mirror, not a promotion gap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PhaseBWalkState {
    counters: HashMap<Symbol, CounterStack>,
    strings: HashMap<Symbol, NamedStringState>,
    running: HashMap<Symbol, RunningTemplateId>,
}

impl PhaseBWalkState {
    /// Apply one [`GcpmDirective`] to this walk state.
    ///
    /// `RegisterTarget` is explicitly **not** handled here — bd
    /// raikiri-spike-0nyv owns `TargetRegistry` / `RegisterTarget` wiring
    /// (a sibling, concurrently-dispatched task); this walk treats it as a
    /// documented no-op pass-through. [`GcpmDirective`] is
    /// `#[non_exhaustive]` cross-crate, so a trailing wildcard arm covers
    /// any future variant the same way (no-op, not a panic) until this walk
    /// is deliberately extended to handle it.
    ///
    /// **`RegisterRunning` has no producer yet.** This arm's rebind
    /// semantics are handled and unit tested, but
    /// `crate::running::collect_running_template` (bd raikiri-spike-e81n)
    /// deliberately does *not* emit `RegisterRunning` for nested
    /// `position: running(name)` seeds inside a template subtree — that
    /// "if the spec/impl allows" hedge is left open on purpose (fail-closed,
    /// 原則3; see that function's doc and
    /// [`crate::running::ParsedRunningTemplate`]'s type-level `directives`
    /// note). So on the only production path into this walk today
    /// ([`apply_running_template_directives`]), this arm is unreachable —
    /// `self.running` is currently populated only by this module's direct
    /// unit tests, not by any real producer.
    pub(crate) fn apply_directive(&mut self, directive: &GcpmDirective) {
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
                // Freeze the whole counters map now — see
                // StringSnapshot::counter_snapshot's doc for why this must
                // happen at apply time, not deferred to read time.
                let counter_snapshot = self
                    .counters
                    .iter()
                    .map(|(k, v)| (k.clone(), v.values().to_vec()))
                    .collect();
                let snapshot = StringSnapshot {
                    source: source.clone(),
                    counter_snapshot,
                };
                self.strings
                    .entry(name.clone())
                    .or_default()
                    .apply_string_set(snapshot);
            }
            GcpmDirective::RegisterRunning { name, template_id } => {
                // Insert-or-overwrite — a later RegisterRunning for the same
                // name rebinds it to the new template_id (last-wins).
                self.running.insert(name.clone(), *template_id);
            }
            GcpmDirective::RegisterTarget { .. } => {
                // Owned by bd raikiri-spike-0nyv (TargetRegistry producer) —
                // deliberately not built here, see this method's doc.
            }
            // cov:ignore: cross-crate `#[non_exhaustive]` catch-all — stable
            // Rust requires the `_` arm for exhaustive matching on
            // GcpmDirective defined in raikiri-traits; unreachable until
            // raikiri-traits adds a 7th variant this walk hasn't been
            // extended to handle.
            _ => {}
        }
    }

    /// Page-boundary hook — forwards to every tracked [`NamedStringState`].
    /// See [`NamedStringState::begin_page`] for why this has no production
    /// caller yet.
    #[allow(
        dead_code,
        reason = "No production per-page driver exists yet (see module-level \
                  doc) — exercised directly by this module's unit tests."
    )]
    pub(crate) fn begin_page(&mut self) {
        for state in self.strings.values_mut() {
            state.begin_page();
        }
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn counter(&self, name: &Symbol) -> Option<&CounterStack> {
        self.counters.get(name)
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn string_state(&self, name: &Symbol) -> Option<&NamedStringState> {
        self.strings.get(name)
    }

    #[allow(
        dead_code,
        reason = "No production reader yet — exercised directly by this \
                  module's unit tests, exposed for a future consumer."
    )]
    pub(crate) fn running_binding(&self, name: &Symbol) -> Option<RunningTemplateId> {
        self.running.get(name).copied()
    }
}

/// Apply every directive in a [`crate::running::ParsedRunningTemplate`]'s
/// `directives` list to `state`, front-to-back.
///
/// "Front-to-back, not re-sorted" matters: `collect_running_template` (bd
/// raikiri-spike-e81n) pushes same-element directives in CSS Lists 3 §4
/// processing order (reset → increment → set, *not* property declaration
/// order — see that function's doc comment), specifically so a consumer
/// walking the Vec in push order gets correct same-element semantics without
/// re-sorting by directive kind. This function is that consumer.
///
/// Current caller: this module's own unit tests. No production per-page
/// driver exists yet — a later, separate wall/dom-paint task (see
/// module-level doc) — so this is exercised via unit tests until then, the
/// same status [`crate::running::collect_running_template`] and its sibling
/// helpers already carry.
#[allow(
    dead_code,
    reason = "No production per-page driver exists yet (a later, separate \
              wall/dom-paint task) — exercised via unit tests until then, \
              same status crate::running::collect_running_template and its \
              siblings already carry."
)]
pub(crate) fn apply_running_template_directives(
    state: &mut PhaseBWalkState,
    template: &crate::running::ParsedRunningTemplate,
) {
    for directive in &template.directives {
        state.apply_directive(directive);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{ContentValueItem, NodeId};

    fn literal_snapshot(text: &str) -> StringSnapshot {
        StringSnapshot {
            source: ContentSource::new(vec![ContentValueItem::Literal(text.to_owned())]),
            counter_snapshot: HashMap::new(),
        }
    }

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
        fn reset_creates_scope_with_given_value() {
            let mut stack = CounterStack::default();
            stack.reset(5);
            assert_eq!(stack.current(), Some(5));
        }

        #[test]
        fn reset_pushes_new_nested_scope_not_replace() {
            // Regression pin: reset must PUSH, not overwrite — two resets
            // for the same name (e.g. a nested <ol><li><ol>...) must both be
            // observable via `values()`, not collapse to one frame.
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
            assert_eq!(stack.values(), &[0, 15], "outer frame must be untouched");
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
            assert_eq!(stack.values(), &[0, 99], "outer frame must be untouched");
        }

        #[test]
        fn pop_scope_removes_only_innermost_frame_and_reveals_previous() {
            let mut stack = CounterStack::default();
            stack.reset(1);
            stack.reset(2);
            assert_eq!(stack.pop_scope(), Some(2));
            assert_eq!(stack.current(), Some(1), "popping reveals the outer scope");
            assert_eq!(stack.pop_scope(), Some(1));
            assert_eq!(stack.current(), None);
        }

        #[test]
        fn pop_scope_on_empty_stack_returns_none_without_panicking() {
            let mut stack = CounterStack::default();
            assert_eq!(stack.pop_scope(), None);
        }

        #[test]
        fn values_are_ordered_outermost_first() {
            let mut stack = CounterStack::default();
            stack.reset(1);
            stack.reset(2);
            stack.reset(3);
            assert_eq!(stack.values(), &[1, 2, 3]);
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
            state.apply_string_set(literal_snapshot("A"));
            let a = literal_snapshot("A");
            assert_eq!(state.on_page_first_use(), Some(&a));
            assert_eq!(state.on_page_last_use(), Some(&a));
            assert_eq!(state.running(), Some(&a));
            // begin_page has never run — no page-entry value exists yet.
            assert_eq!(state.on_page_start(), None);
        }

        #[test]
        fn second_string_set_same_page_updates_last_not_first() {
            let mut state = NamedStringState::default();
            state.apply_string_set(literal_snapshot("A"));
            state.apply_string_set(literal_snapshot("B"));
            assert_eq!(
                state.on_page_first_use(),
                Some(&literal_snapshot("A")),
                "first-use must stay pinned to the first assignment on the page"
            );
            assert_eq!(state.on_page_last_use(), Some(&literal_snapshot("B")));
            assert_eq!(state.running(), Some(&literal_snapshot("B")));
        }

        #[test]
        fn begin_page_before_any_string_set_leaves_start_none() {
            // Document-start case: no entry value exists yet because
            // nothing has ever been assigned.
            let mut state = NamedStringState::default();
            state.begin_page();
            assert_eq!(state.on_page_start(), None);
        }

        #[test]
        fn begin_page_snapshots_running_into_start_and_clears_first_last() {
            let mut state = NamedStringState::default();
            state.apply_string_set(literal_snapshot("A"));
            state.begin_page();
            assert_eq!(
                state.on_page_start(),
                Some(&literal_snapshot("A")),
                "entry value carried forward from the previous page"
            );
            assert_eq!(
                state.on_page_first_use(),
                None,
                "fresh page has no assignments yet"
            );
            assert_eq!(state.on_page_last_use(), None);
            assert_eq!(
                state.running(),
                Some(&literal_snapshot("A")),
                "running is unaffected by begin_page — only snapshotted from"
            );
        }

        #[test]
        fn multi_page_sequence_tracks_snapshot_timing_and_ordering() {
            // page 1: single assignment "A"
            let mut state = NamedStringState::default();
            state.apply_string_set(literal_snapshot("A"));

            // → page 2 starts; entry value carried from page 1's "A"
            state.begin_page();
            assert_eq!(state.on_page_start(), Some(&literal_snapshot("A")));
            assert_eq!(state.on_page_first_use(), None);
            assert_eq!(state.on_page_last_use(), None);

            // page 2: two assignments "B" then "C"
            state.apply_string_set(literal_snapshot("B"));
            state.apply_string_set(literal_snapshot("C"));
            assert_eq!(state.on_page_first_use(), Some(&literal_snapshot("B")));
            assert_eq!(state.on_page_last_use(), Some(&literal_snapshot("C")));
            assert_eq!(state.running(), Some(&literal_snapshot("C")));

            // → page 3 starts; entry value carried from page 2's last "C"
            state.begin_page();
            assert_eq!(state.on_page_start(), Some(&literal_snapshot("C")));
            assert_eq!(state.on_page_first_use(), None);
            assert_eq!(state.on_page_last_use(), None);
            // running still reflects the last real assignment, unaffected
            // by the page boundary.
            assert_eq!(state.running(), Some(&literal_snapshot("C")));
        }
    }

    // ── PhaseBWalkState ──────────────────────────────────

    mod phase_b_walk_state_tests {
        use super::*;

        #[test]
        fn counter_directives_dispatch_to_named_stack() {
            let mut state = PhaseBWalkState::default();
            let foo = Symbol::new("foo");
            let bar = Symbol::new("bar");
            state.apply_directive(&GcpmDirective::CounterReset {
                name: foo.clone(),
                value: 0,
            });
            state.apply_directive(&GcpmDirective::CounterIncrement {
                name: foo.clone(),
                delta: 3,
            });
            state.apply_directive(&GcpmDirective::CounterSet {
                name: bar.clone(),
                value: 9,
            });

            assert_eq!(state.counter(&foo).and_then(CounterStack::current), Some(3));
            assert_eq!(state.counter(&bar).and_then(CounterStack::current), Some(9));
        }

        #[test]
        fn same_element_reset_increment_set_order_matches_e81n_push_order() {
            // Pin against a re-sort-by-kind bug: collect_running_template
            // pushes reset → increment → set for the SAME element (CSS
            // Lists 3 §4 processing order), and this walk must apply that
            // sequence front-to-back, not group/re-sort by directive kind.
            let mut state = PhaseBWalkState::default();
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
                state.apply_directive(d);
            }
            // set (5) applied after increment (0+1=1) must win — final = 5.
            assert_eq!(state.counter(&c).and_then(CounterStack::current), Some(5));
        }

        #[test]
        fn register_running_inserts_binding() {
            let mut state = PhaseBWalkState::default();
            let name = Symbol::new("header");
            let id = RunningTemplateId::new(NodeId::new(7));
            state.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id,
            });
            assert_eq!(state.running_binding(&name), Some(id));
        }

        #[test]
        fn register_running_rebind_overwrites_previous_binding() {
            let mut state = PhaseBWalkState::default();
            let name = Symbol::new("header");
            let id1 = RunningTemplateId::new(NodeId::new(7));
            let id2 = RunningTemplateId::new(NodeId::new(42));
            state.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id1,
            });
            state.apply_directive(&GcpmDirective::RegisterRunning {
                name: name.clone(),
                template_id: id2,
            });
            assert_eq!(
                state.running_binding(&name),
                Some(id2),
                "a later RegisterRunning for the same name must rebind, not be ignored"
            );
        }

        #[test]
        fn register_target_is_a_documented_no_op() {
            let mut before = PhaseBWalkState::default();
            before.apply_directive(&GcpmDirective::CounterReset {
                name: Symbol::new("x"),
                value: 1,
            });
            let mut after = before.clone();
            after.apply_directive(&GcpmDirective::RegisterTarget {
                fragment_id: Symbol::new("intro"),
            });
            assert_eq!(
                before, after,
                "RegisterTarget must not mutate counters/strings/running — owned by 0nyv"
            );
        }

        #[test]
        fn string_set_freezes_counter_values_at_apply_time() {
            let mut state = PhaseBWalkState::default();
            let chapter = Symbol::new("chapter");
            let title = Symbol::new("title");

            state.apply_directive(&GcpmDirective::CounterReset {
                name: chapter.clone(),
                value: 3,
            });
            state.apply_directive(&GcpmDirective::StringSet {
                name: title.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("Ch.".into())]),
            });

            // Mutate the counter AFTER the string-set fired.
            state.apply_directive(&GcpmDirective::CounterIncrement {
                name: chapter.clone(),
                delta: 100,
            });

            let snapshot = state
                .string_state(&title)
                .and_then(NamedStringState::running)
                .expect("string-set applied");
            assert_eq!(
                snapshot.counter_snapshot.get(&chapter),
                Some(&vec![3]),
                "counter_snapshot must reflect the value AT string-set time (3), \
                 not the later mutation (103) — evaluated-in-place semantics"
            );
            // Live counter state, meanwhile, does reflect the later mutation.
            assert_eq!(
                state.counter(&chapter).and_then(CounterStack::current),
                Some(103)
            );
        }

        #[test]
        fn begin_page_forwards_to_every_tracked_named_string() {
            let mut state = PhaseBWalkState::default();
            let a = Symbol::new("a");
            let b = Symbol::new("b");
            state.apply_directive(&GcpmDirective::StringSet {
                name: a.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("A".into())]),
            });
            state.apply_directive(&GcpmDirective::StringSet {
                name: b.clone(),
                source: ContentSource::new(vec![ContentValueItem::Literal("B".into())]),
            });

            state.begin_page();

            assert!(state.string_state(&a).unwrap().on_page_start().is_some());
            assert!(state.string_state(&b).unwrap().on_page_start().is_some());
            assert!(
                state
                    .string_state(&a)
                    .unwrap()
                    .on_page_first_use()
                    .is_none()
            );
            assert!(
                state
                    .string_state(&b)
                    .unwrap()
                    .on_page_first_use()
                    .is_none()
            );
        }
    }

    // ── apply_running_template_directives ────────────────

    mod apply_running_template_directives_tests {
        use super::*;
        use crate::running::ParsedRunningTemplate;
        use std::sync::Arc;

        #[test]
        fn applies_every_directive_front_to_back_and_skips_register_target() {
            let name = Symbol::new("chapter");
            let running_name = Symbol::new("header");
            let template_id = RunningTemplateId::new(NodeId::new(99));
            let directives = vec![
                GcpmDirective::CounterReset {
                    name: name.clone(),
                    value: 0,
                },
                GcpmDirective::CounterIncrement {
                    name: name.clone(),
                    delta: 1,
                },
                GcpmDirective::StringSet {
                    name: Symbol::new("title"),
                    source: ContentSource::new(vec![ContentValueItem::Literal("Ch. 1".into())]),
                },
                GcpmDirective::RegisterRunning {
                    name: running_name.clone(),
                    template_id,
                },
                GcpmDirective::RegisterTarget {
                    fragment_id: Symbol::new("intro"),
                },
            ];
            // Field values beyond `directives` are irrelevant to this
            // function; use crate::running's own test-shape defaults.
            let template = ParsedRunningTemplate {
                subtree_root: NodeId::new(1),
                computed_styles: Arc::default(),
                directives,
                dynamic_flags: Default::default(),
            };

            let mut state = PhaseBWalkState::default();
            apply_running_template_directives(&mut state, &template);

            assert_eq!(
                state.counter(&name).and_then(CounterStack::current),
                Some(1)
            );
            assert_eq!(state.running_binding(&running_name), Some(template_id));
            assert!(
                state.string_state(&Symbol::new("title")).is_some(),
                "StringSet must have been applied"
            );
        }
    }
}
