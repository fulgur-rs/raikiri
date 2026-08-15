//! GCPM `position: running(name)` — `RunningTemplateStore` (tier-1 cache) +
//! `element(name)` runtime name→template resolve + per-page re-layout hook.
//!
//! **Ownership** (design §7.0 / §7.3): raikiri-dom (runtime side) owns the
//! parsed running template pool. raikiri-style (static side) already emits
//! [`raikiri_style::computed::RunningTemplate`] as the per-node cascade seed
//! (name only, no subtree identity). This module receives those seeds and
//! associates each with a DOM subtree root, its pre-cascaded style block, the
//! GCPM directives that live under it, and dynamic-content flags — that
//! collection is the "parsed template" the design doc calls out. The @page
//! margin box's `content: element(name, [first|start|last|first-except]?)` is
//! answered by [`RunningTemplateStore::resolve_element_pool`] (name →
//! ordered pool of registered assignments); the optional selector keyword
//! semantics (default `first` per CSS GCPM 3 §1.2.2
//! <https://www.w3.org/TR/css-gcpm-3/#element-syntax>) is applied by the
//! caller (PageStream), which has the page/geometry context needed to filter
//! the pool. Per-page re-layout is invoked via [`layout_running_template`].
//!
//! **Canonical shape** (design §7.3, lines 1975-1995):
//! ```text
//! struct RunningTemplateStore {
//!     parsed_templates: HashMap<RunningTemplateId, ParsedRunningTemplate>,
//!     // layout 結果はキャッシュしない
//! }
//!
//! struct ParsedRunningTemplate {
//!     subtree_root: NodeId,
//!     computed_styles: Arc<CascadeSubset>,
//!     directives: Vec<GcpmDirective>,
//!     dynamic_flags: DynamicFlags,
//! }
//!
//! struct DynamicFlags {
//!     has_counter: bool,
//!     has_string: bool,
//!     has_target: bool,
//!     has_content_variant: bool,
//! }
//! ```
//!
//! **"2-tier" is one cache today** — the design's tier-1 is
//! `parsed_templates` (this module); tier-2 is a future layout-result cache
//! for fully-static templates (`dynamic_flags = all false`) and is
//! **explicitly deferred** (§7.3 lines 2014-2016: always re-layout for now,
//! no result cache). Per-page re-layout via [`layout_running_template`] is
//! the current flow (§7.3 lines 2007-2012).
//!
//! **Divergence from canonical shape** — same "pub(crate) local until
//! traits reconciliation decision" convention that the
//! `crate::target` (removed) module previously held (that reconciliation has
//! since landed — the TargetRegistry /
//! TargetInfo / ResolveOutcome / PendingResolution / resolve_content_component
//! API now lives at [`raikiri_traits::TargetRegistry`] and friends). This module
//! currently writes `pub(crate)` and does NOT re-export through the crate
//! root; a later reconciliation task will decide whether any of these types
//! need to cross wall/traits or wall/dom-paint (the
//! [`raikiri_traits::GcpmDirective`] variant populate has already landed; a
//! parallel promotion for RunningTemplate is not yet scheduled).
//!
//! **`RunningTemplateId` = subtree_root [`NodeId`]** (canonical: per-element
//! unique). The design doc's `HashMap<RunningTemplateId, ParsedRunningTemplate>`
//! shape signals a distinct identifier type — CSS GCPM 3 §1.2.2's
//! `element(<custom-ident>, [first|start|last|first-except]?)` grammar
//! (<https://www.w3.org/TR/css-gcpm-3/#element-syntax>) requires retaining
//! *every* element registered under `position: running(name)` in document
//! order because the selector keyword (default `first`) is applied per page
//! over the full pool; collapsing at registration would foreclose the
//! `start`/`last`/`first-except` variants. Keying `parsed_templates` by name
//! would also collapse multi-element documents to their last registration.
//! Keying by the element's `NodeId` (the subtree root, unique in the arena)
//! preserves the pool; a sibling `name_pool` index maps each name to its
//! ordered list of ids. The per-page selector-keyword filter stays on the
//! PageStream / paint driver side — a later, separate wall/dom-paint task
//! (see `MarginBoxGeometry` / `layout_running_template` below for the
//! dom-internal boundary that task will consume). That paint-side driver
//! doesn't exist yet — see
//! [`RunningTemplateStore::resolve_first`]'s doc for the closest dom-side
//! landing (pool-order lookup, explicitly NOT the spec's page-relative
//! keyword semantics) and why the gap stops there.
//!
//! **`CascadeSubset` local definition** — the design doc names
//! `computed_styles: Arc<CascadeSubset>` but no such type exists in the
//! workspace today. Rather than reach into raikiri-style for a shape that
//! doesn't fit ([`raikiri_style::computed::ComputedValues`] is per-node,
//! `CascadeSubset` is a per-subtree collection) or reach into raikiri-traits
//! to promote a placeholder (wall/traits crossing), this module defines
//! [`CascadeSubset`] as a `pub(crate)` local. This is the same "carry a
//! concrete local type until a cross-crate reconciliation is scheduled"
//! shape that [`raikiri_traits::TargetInfo`] previously held before its own
//! reconciliation landed (a wall/traits merge) — CascadeSubset awaits a
//! similar promotion.
//!
//! **`ContentComponent::Element { name }` upstream gap** — the css-engine
//! side of `content: element(name)` (parsing the `element(<name>)`
//! functional-notation into a `ContentComponent::Element` variant on
//! [`raikiri_style::property::ContentComponent`]) does NOT exist today; the
//! enum has no `Element` arm. Adding it is a css-engine concern
//! (wall/css-engine, not this task's walls). This module therefore lands the
//! **dom-side deliverable** — name→pool→[`ParsedRunningTemplate`] lookup, via
//! [`RunningTemplateStore::resolve_first`] —
//! which is the actual "element(name) resolve" once the caller has extracted
//! the name from wherever. `resolve_first` composes
//! [`RunningTemplateStore::resolve_element_pool`] +
//! [`RunningTemplateStore::get`] but does NOT implement the CSS GCPM 3
//! §1.2.2 selector-keyword semantics (`first`/`start`/`last`/`first-except`)
//! — see `resolve_first`'s own doc for why that stays page-context-dependent
//! and out of this store's reach. When the css-engine adds the
//! `ContentComponent::Element` variant, a driver similar to
//! [`raikiri_traits::resolve_content_component`] can wire ContentComponent →
//! `resolve_first` (or the full pool, for keyword-aware callers) in a single
//! call — this remains blocked on that css-engine-side variant landing.
//!
//! **Registration order == document order** — the store assumes the caller
//! invokes [`RunningTemplateStore::register`] in DOM tree order.
//! [`build_running_template_store`] walks the arena
//! in document order, so this assumption holds automatically; if it is ever
//! violated, per-page pool selection (any selector-keyword variant) would
//! emit the wrong element and visible layout would drift. Regression pin:
//! the `pool_preserves_registration_order` unit test (store-level) and
//! `build_running_template_store_registers_siblings_in_document_order`
//! (walker-level).
//!
//! **Single-name-per-id invariant** — a given subtree_root [`NodeId`] must be
//! registered under a single running-template name for the lifetime of the
//! store. The register-site walker walks each element once, so an element
//! carries a single `position: running(<name>)` computed value; the store
//! reconciles same-id-different-name re-registration by moving the id
//! between name pools to keep the index self-consistent (see
//! [`RunningTemplateStore::register`]). Regression pin: the
//! `re_register_same_id_under_different_name_transfers_pool_entry` unit test.
//!
//! **Divergence from [`raikiri_traits::TargetRegistry`]'s register policy** —
//! `TargetRegistry` is first-wins per DOM id-resolution
//! (`getElementById` first-in-tree-order). This store retains all
//! registrations per name (no wins/loses on registration; selection is a
//! per-page later concern). Different spec, different shape.

use std::collections::HashMap;
use std::sync::Arc;

use raikiri_style::CascadeResult;
use raikiri_style::property::{ContentComponent, ContentTextKeyword};
use raikiri_traits::{
    ContentSource, ContentValueItem, GcpmDirective, NodeId, NodeKind, RunningTemplateId, Symbol,
};
use smol_str::SmolStr;

use crate::document::Document;
use crate::node::NodeData;

// `RunningTemplateId` is the shared identifier from raikiri-traits
// (design §7.0 line 1904 "shared types → raikiri-traits"). Landed together
// with the `GcpmDirective` populate — the
// `RegisterRunning` variant references it (§7.1 line 1918). Previously this
// module carried a `pub(crate)` local mirror; that mirror is dropped now that
// the traits-side canonical location exists.

/// Per-subtree pre-cascaded style block.
///
/// Design doc §7.3 names this `Arc<CascadeSubset>` without pinning a concrete
/// shape; the natural fit for "the styles under a running-template subtree" is
/// a walk-order [`Vec`] of per-node [`raikiri_style::ComputedValues`] (the
/// same shape [`raikiri_style::CascadeResult::computed`] already exposes for
/// the whole document, but scoped to the subtree). The [`Arc`] wrap keeps the
/// outer clone O(1) once the template is registered — running templates are
/// looked up per-page and per-margin-box, so the store's `clone()` bump path
/// must not deep-copy the style block.
#[derive(Debug, Default)]
pub(crate) struct CascadeSubset {
    /// Per-node computed styles in subtree walk order (parallel to the DOM
    /// walk from `subtree_root`, matching `CascadeResult::computed`'s layout).
    #[allow(
        dead_code,
        reason = "Written by collect_running_template; \
                  read by the per-page PageStream / paint driver \
                  (layout_running_template's future real body), a later \
                  wall/dom-paint task — see collect_running_template's \
                  Non-goals section."
    )]
    pub(crate) styles: Vec<raikiri_style::ComputedValues>,
}

/// Dynamic-content flags per running template — the four axes that make a
/// template's rendering depend on per-page state (design §7.3 lines
/// 1989-1993).
///
/// A template whose flags are all `false` is fully static and would qualify
/// for a future layout-result cache; today every template is
/// re-laid-out per page regardless, so the flags are captured as
/// **observability** for the eventual optimization — not consulted by
/// [`layout_running_template`].
///
/// **`has_content_variant` semantics** — matches the design doc
/// parenthetical (§7.3 line 1993 "content(before/after) 参照あり"). Flipped
/// on `content(before)` and `content(after)` because pseudo-element string
/// values are themselves built from `::before` / `::after` `content:`
/// content-lists that may contain page-dependent bits (`counter()`,
/// `string()`, `target-*`) — so `content(before)` transitively depends on
/// per-page state. `content(text)` and `content(first-letter)` are NOT
/// flipped: they read the element's own text content, which is fixed per
/// running-element. Since a future static-template layout cache would key
/// on `(template_id, effective margin-box geometry)`, distinct running
/// elements (chapter-1's `<h1>` vs chapter-2's `<h1>`) naturally miss the
/// cache via distinct `template_id`s — the `content(text)` axis does not
/// add per-page dynamism the id-key doesn't already cover. The `#[non_exhaustive]`
/// catch-all in `detect_dynamic_flags`'s Content arm over-marks unknown
/// future keywords for safety (§7.3 line 2005 "correctness 優先").
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DynamicFlags {
    /// `counter()` / `counters()` reference present anywhere in the template.
    pub(crate) has_counter: bool,
    /// `string()` reference present anywhere in the template.
    pub(crate) has_string: bool,
    /// `target-counter()` / `target-counters()` / `target-text()` reference
    /// present anywhere in the template.
    pub(crate) has_target: bool,
    /// `content(before)` / `content(after)` reference present anywhere in
    /// the template. Named "content variant" because the pseudo-element
    /// content-list (which `content(before|after)` reads) may itself contain
    /// page-dependent bits (`counter()`, `string()`, `target-*`). See
    /// type-level note for why `content(text)` / `content(first-letter)` do
    /// NOT flip this flag.
    pub(crate) has_content_variant: bool,
}

impl DynamicFlags {
    /// `true` iff every axis is `false` — the template is fully static and
    /// would qualify for a future layout-result cache.
    pub(crate) fn is_fully_static(self) -> bool {
        !self.has_counter && !self.has_string && !self.has_target && !self.has_content_variant
    }
}

/// A running-template subtree, pre-cascaded and analyzed once at registration
/// time.
///
/// Design doc §7.3 line 1982-1987: the tier-1 cache entry — a DOM subtree
/// root + pre-cascaded style block + GCPM directives + dynamic-content flags.
/// Layout is NOT cached here (§7.3 line 1979 "layout 結果はキャッシュしない");
/// per-page re-layout runs via [`layout_running_template`].
///
/// **`directives` field** — [`raikiri_traits::GcpmDirective`] was uninhabited
/// until the variant populate landed (canonical 6-variant shape per design
/// doc §7.1 line 1913-1920). [`collect_running_template`]
/// populates the field with `CounterIncrement` / `CounterReset` /
/// `CounterSet` / `StringSet` records read off every node's
/// [`raikiri_style::ComputedValues`] in the subtree. It does **not** emit
/// `RegisterRunning` for nested `position: running(name)` seeds inside the
/// subtree (the "if the spec/impl allows" hedge below is deliberately left
/// unresolved — fail-closed, 原則 3) nor `RegisterTarget` (that directive is
/// `id`-attribute-driven and belongs to the separate
/// [`raikiri_traits::TargetRegistry`] registration walk, a different task's
/// territory — see the module-level "Divergence from
/// `raikiri_traits::TargetRegistry`'s register policy" note).
#[derive(Debug)]
pub(crate) struct ParsedRunningTemplate {
    /// The subtree root inside the document arena. `position: running(name)`
    /// removes this element from body flow; the pool holds the pointer.
    /// Doubles as the [`RunningTemplateId`] key
    /// ([`RunningTemplateId::new(subtree_root)`](RunningTemplateId::new)).
    pub(crate) subtree_root: NodeId,
    /// Pre-cascaded style block for every node under `subtree_root` (walk
    /// order). See [`CascadeSubset`].
    #[allow(
        dead_code,
        reason = "Written by collect_running_template; \
                  read by the per-page PageStream / paint driver, a later \
                  wall/dom-paint task — see collect_running_template's \
                  Non-goals section."
    )]
    pub(crate) computed_styles: Arc<CascadeSubset>,
    /// GCPM directives that live inside the template subtree
    /// (`counter-increment`, `counter-reset`, `counter-set`, `string-set`,
    /// nested `running()` seeds if the spec/impl allows). Populated by
    /// [`collect_running_template`]; see the
    /// type-level `directives` field note for exactly what is (and isn't)
    /// emitted.
    #[allow(
        dead_code,
        reason = "Written by collect_running_template; \
                  read by crate::gcpm::PhaseBWalkState::apply_directive via \
                  crate::gcpm::apply_running_template_directives (landed — \
                  the dom-local Phase B walk \
                  state/algorithm; promotion onto raikiri_traits::PageContext \
                  is a separate, still-blocked follow-up). No production \
                  per-page driver invokes that walk \
                  yet (a later, separate wall/dom-paint task) — exercised via \
                  unit tests until then."
    )]
    pub(crate) directives: Vec<GcpmDirective>,
    /// Which dynamic axes this template exercises (see [`DynamicFlags`]).
    pub(crate) dynamic_flags: DynamicFlags,
}

/// Tier-1 running-template store — parsed templates keyed by per-element
/// [`RunningTemplateId`], plus a name→ordered-pool index for `element(name)`
/// resolves.
///
/// Design doc §7.3 lines 1976-1980. Layout results are NOT cached; per-page
/// re-layout is unconditional today
/// ([`layout_running_template`]). A future tier-2 layout-result cache
/// would slot in alongside — this file does not implement it.
///
/// **Keying rationale**: `parsed_templates` is keyed by [`RunningTemplateId`]
/// (per-element id, wraps the subtree root's [`NodeId`]). The sibling
/// `name_pool` index maps each running-template name to the ids of every
/// element registered under `position: running(name)`, in **registration
/// (document) order**. Per-page selection — applying the
/// `element(<name>, [first|start|last|first-except]?)` selector keyword per
/// CSS GCPM 3 §1.2.2 (<https://www.w3.org/TR/css-gcpm-3/#element-syntax>) —
/// is a PageStream concern — a later, separate wall/dom-paint task; this
/// store hands the full ordered pool over and lets the caller filter. See
/// the module-level
/// "`RunningTemplateId` = subtree_root" note for the shape justification.
///
/// See the module-level "Divergence from canonical shape" note for the
/// `pub(crate)` scoping rationale.
#[derive(Debug, Default)]
pub(crate) struct RunningTemplateStore {
    /// Per-element unique-id → parsed template (design canonical shape:
    /// `HashMap<RunningTemplateId, ParsedRunningTemplate>`).
    parsed_templates: HashMap<RunningTemplateId, ParsedRunningTemplate>,
    /// Name → ordered pool of registered ids. Order == registration order,
    /// which the caller (register-site walker) guarantees is document order.
    /// Non-canonical additive index; see module-level shape note.
    name_pool: HashMap<Symbol, Vec<RunningTemplateId>>,
    /// Reverse index — id → the name it is currently registered under.
    /// Non-canonical additive index used by [`Self::register`] to detect
    /// same-id-different-name re-registration and transfer the id between
    /// name pools atomically (without this index, an id registered under
    /// name A then re-registered under name B would leave a stale entry in
    /// A's pool and never land in B's pool).
    id_to_name: HashMap<RunningTemplateId, Symbol>,
}

impl RunningTemplateStore {
    /// Register a parsed running template under `name`.
    ///
    /// Appends the template's id to `name`'s ordered pool and inserts the
    /// template into `parsed_templates`. **All prior registrations under the
    /// same name are retained** — CSS GCPM 3 §1.2.2 "The element() value"
    /// (<https://www.w3.org/TR/css-gcpm-3/#element-syntax>) syntax
    /// `element(<custom-ident>, [first|start|last|first-except]?)` requires
    /// per-page selection (default `first`) over the full ordered pool;
    /// collapsing at registration would foreclose the `start`/`last`/
    /// `first-except` variants and lose the pool multi-element documents
    /// need.
    ///
    /// This differs from [`raikiri_traits::TargetRegistry::register`]'s
    /// first-wins policy — that's DOM id-resolution (`getElementById`,
    /// unique-id semantics); this is per-page pool selection. Different
    /// spec, different shape.
    ///
    /// **Caller invariant — document-order registration.** The store assumes
    /// the caller invokes `register` in document order
    /// ([`build_running_template_store`] walks the arena in document order,
    /// so this holds automatically). If violated, per-page pool selection
    /// emits the wrong element.
    ///
    /// **Re-registration semantics** — determined by whether the id (=
    /// `template.subtree_root`) was previously seen:
    /// - **New id**: appended to `name`'s pool; recorded in the id→name
    ///   reverse index.
    /// - **Existing id, same name**: template replaced in place, pool order
    ///   preserved (the id remains at its original position in the pool).
    /// - **Existing id, different name**: the id is *transferred* — removed
    ///   from the previous name's pool (which is deleted from the index if
    ///   emptied) and appended to the new name's pool. The reverse index is
    ///   updated. This case is not expected during normal walker operation
    ///   (the caller invariant is one name per element, since
    ///   `position: running(<name>)` is a single computed value on the
    ///   element), but the transfer keeps the store self-consistent under
    ///   repeated cascade or an unforeseen recursive walker. Regression pin:
    ///   `re_register_same_id_under_different_name_transfers_pool_entry`.
    pub(crate) fn register(
        &mut self,
        name: Symbol,
        template: ParsedRunningTemplate,
    ) -> RunningTemplateId {
        let id = RunningTemplateId::new(template.subtree_root);
        // Reconcile the name index for the three re-registration cases (see
        // doc comment). The reverse index (`id_to_name`) is authoritative
        // for "which pool is this id currently in".
        match self.id_to_name.get(&id).cloned() {
            None => {
                // Fresh id — append to the name pool and record the reverse
                // mapping.
                self.name_pool.entry(name.clone()).or_default().push(id);
                self.id_to_name.insert(id, name);
            }
            Some(prev_name) if prev_name == name => {
                // Same id, same name — nothing to reconcile; the pool
                // already carries the id in its original position.
            }
            Some(prev_name) => {
                // Same id, different name — transfer between pools.
                if let Some(pool) = self.name_pool.get_mut(&prev_name) {
                    pool.retain(|slot| *slot != id);
                    if pool.is_empty() {
                        self.name_pool.remove(&prev_name);
                    }
                }
                self.name_pool.entry(name.clone()).or_default().push(id);
                self.id_to_name.insert(id, name);
            }
        }
        // Template payload is always overwritten (last-wins per id).
        self.parsed_templates.insert(id, template);
        id
    }

    /// Resolve `content: element(name, ...)` — return the ordered pool of
    /// registered template ids under `name` (document order).
    ///
    /// Returns an empty slice when the name has no registrations. **Per-page
    /// selection is the caller's responsibility**: CSS GCPM 3 §1.2.2's
    /// `element(<name>, [first|start|last|first-except]?)` syntax
    /// (<https://www.w3.org/TR/css-gcpm-3/#element-syntax>) applies a
    /// per-page selector keyword (default `first`) over the pool this method
    /// returns. That filter requires page/geometry context, which lives on
    /// the PageStream side (design §9.1); this store's job ends at handing
    /// over the pool.
    ///
    /// An unresolvable `element(name)` (empty pool) yields empty content per
    /// spec — this store returns `&[]` and lets the caller decide the empty
    /// fallback (mirrors [`raikiri_traits::TargetRegistry`]'s non-fragment
    /// empty-string fallback).
    pub(crate) fn resolve_element_pool(&self, name: &Symbol) -> &[RunningTemplateId] {
        self.name_pool.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Fetch the parsed template for an id (typically an id drawn from
    /// [`Self::resolve_element_pool`]).
    pub(crate) fn get(&self, id: RunningTemplateId) -> Option<&ParsedRunningTemplate> {
        self.parsed_templates.get(&id)
    }

    /// Compose [`Self::resolve_element_pool`] and [`Self::get`] into the
    /// minimal dom-internal `content: element(name)` lookup: the
    /// first-registered (== first document-order) template under `name`.
    ///
    /// **Not GCPM 3 §1.2.2 keyword semantics.** §1.2.2 ("The element()
    /// value", <https://www.w3.org/TR/css-gcpm-3/#element-syntax>) supplies
    /// the grammar (`element(<custom-ident>, [first|start|last|
    /// first-except]?)`) but not the keyword's meaning — it says that, just
    /// as with `string()`, `element()` takes an optional keyword to describe
    /// which value should be used when there are multiple assignments on a
    /// page, and defers to `string()`'s own definition. That definition
    /// (§1.1.2 "The string() function",
    /// <https://www.w3.org/TR/css-gcpm-3/#string-first>) is page-relative:
    /// "The value of the first assignment on the page is used. If there is
    /// no assignment on the page, the entry value is used" — where "entry
    /// value" is itself defined as "the assignment in effect at the end of
    /// the previous page." That per-page, carried-forward-from-the-previous-
    /// page semantics needs the page-position context PageStream tracks
    /// (design §9.1); this store has none. `resolve_first` instead returns
    /// the pool-order head (document-first overall) — the only
    /// name→template lookup expressible without page context, and NOT what
    /// the spec's `first` keyword resolves to on any page after the running
    /// element's first assignment. The real `first`/`start`/`last`/
    /// `first-except` filter is the PageStream / paint driver's job — a
    /// later, separate wall/dom-paint task (see the module-level
    /// `RunningTemplateId = subtree_root` note), not this one.
    ///
    /// Returns `None` when `name` has no registrations (mirrors
    /// [`Self::resolve_element_pool`]'s empty-pool contract).
    #[allow(
        dead_code,
        reason = "Real (non-test) caller of resolve_element_pool + get, \
                  already landed; no production call site \
                  yet — the css-engine ContentComponent::Element variant \
                  is the still-open blocker for one."
    )]
    pub(crate) fn resolve_first(&self, name: &Symbol) -> Option<&ParsedRunningTemplate> {
        let id = *self.resolve_element_pool(name).first()?;
        self.get(id)
    }

    /// Number of distinct registered templates.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.parsed_templates.len()
    }
}

/// Detect the four dynamic-content axes in a `content` list.
///
/// Called at parsed-template-registration time to stamp
/// [`DynamicFlags`] onto the [`ParsedRunningTemplate`]. Reads the
/// [`raikiri_style::property::ContentComponent`] variants and flips the
/// corresponding flag; multiple content lists (e.g. one per node in the
/// subtree) are folded via repeated calls with [`DynamicFlags::default`] as
/// the seed and OR-composing — [`collect_running_template`] is the real
/// invocation site doing exactly that fold, one call per subtree node's
/// `content`. `detect_dynamic_flags_composes_multiple_axes` composes several
/// axes within a *single* call's content list; the walker-level
/// `collect_running_template_folds_dynamic_flags_across_subtree_nodes` pins
/// the cross-node OR-fold.
///
/// **`#[non_exhaustive]` handling**: [`ContentComponent`] is
/// `#[non_exhaustive]`; the future `ContentComponent::Element { name }`
/// variant is NOT a dynamic-content signal
/// on its own — `element()` retrieves a static/dynamic template whose
/// dynamism is already captured in that template's own flags. The catch-all
/// arm therefore adds no flag. A downstream variant that would legitimately
/// flip a flag should be handled explicitly here rather than silently
/// falling through the catch-all.
///
/// **`Image` / `Contents` / `Quote` / `Leader`**
/// (CSS Content 3 §2.2 / §2.3 / §2.4.2 / §2.5.1): each now has an explicit
/// no-op arm (parallel to `Literal`/`Attr`) rather than falling through the
/// catch-all — `<image>`'s `url` is fixed per declaration, quote nesting
/// depth is document-structural, and a `leader()` glyph/string is fixed;
/// none read per-page runtime state the way `counter()` / `string()` /
/// `target-*` do.
///
/// `Contents` (`content: contents`) deserves a sharper argument than
/// "page-independent" alone, because it superficially resembles
/// `content(before|after)` (which DOES flip `has_content_variant`, see
/// below) — both "reach into" other content. The difference is *how* that
/// other content enters the fold: `content(before|after)` reads a
/// **pseudo-element's** content-list, which is not itself a subtree node
/// this function (or its caller's per-node walk) ever visits, so any
/// page-dependent bits inside it would otherwise go uncounted — hence the
/// explicit flag. `contents` instead inlines the element's own **DOM
/// descendants**, which per the fold contract above ("multiple content
/// lists — one per node in the subtree — are folded via repeated calls")
/// will be separate arena nodes the register-site walker is contracted to
/// visit and OR-fold independently; their dynamism is meant to be captured
/// directly, not through this arm. So `Contents` genuinely parallels
/// `Literal`/`Attr`, not `Content { keyword: Before | After }`.
///
/// `Leader`'s rendered fill length does vary with available inline space,
/// but that's a layout-geometry input, not a content-dynamism axis — a
/// future cache key would be `(template_id, effective margin-box geometry)`
/// (see the [`DynamicFlags`] type-level note), so geometry variance is
/// already covered by the cache key and doesn't need a flag here.
///
/// **Spec confirmation still pending** on this "no dynamic flag" call —
/// the semantic read above is the implementer's, not yet a
/// spec-verified classification.
pub(crate) fn detect_dynamic_flags(content: &[ContentComponent]) -> DynamicFlags {
    // Fold into local booleans and build the struct at the end — avoids the
    // `field_reassign_with_default` clippy trap that would fire on
    // `let mut flags = default(); ... flags.has_X = true`.
    let mut has_counter = false;
    let mut has_string = false;
    let mut has_target = false;
    let mut has_content_variant = false;
    for cc in content {
        match cc {
            ContentComponent::Counter { .. } | ContentComponent::Counters { .. } => {
                has_counter = true;
            }
            ContentComponent::String { .. } => {
                has_string = true;
            }
            ContentComponent::TargetCounter { .. }
            | ContentComponent::TargetCounters { .. }
            | ContentComponent::TargetText { .. } => {
                has_target = true;
            }
            ContentComponent::Content { keyword } => match keyword {
                ContentTextKeyword::Before | ContentTextKeyword::After => {
                    // ::before / ::after pseudo string values are built from
                    // content-lists that may themselves contain page-
                    // dependent bits (counter / string / target-*). See the
                    // DynamicFlags::has_content_variant type-level note.
                    has_content_variant = true;
                }
                ContentTextKeyword::Text | ContentTextKeyword::FirstLetter => {
                    // Element's own text / first-letter — fixed per running
                    // element; a future layout cache would key on
                    // template_id, and distinct running elements naturally
                    // miss the cache without any dynamic-flag help.
                }
                // ContentTextKeyword is #[non_exhaustive]; a future variant
                // may or may not carry per-page dynamism. Over-mark for
                // safety (§7.3 line 2005 "correctness 優先"); a new keyword
                // should be checked against spec to see whether it deserves
                // an explicit arm.
                _ => {
                    has_content_variant = true;
                }
            },
            ContentComponent::Literal(_) | ContentComponent::Attr { .. } => {
                // Static — contribute no flag.
            }
            ContentComponent::Image { .. }
            | ContentComponent::Contents
            | ContentComponent::Quote(_)
            | ContentComponent::Leader(_) => {
                // Static — page-independent per this function's semantic
                // read (image url() / quote nesting depth / leader glyph
                // resolve without per-page runtime state; `Contents`' own
                // descendants will be separately-walked arena nodes the
                // register-site walker is contracted to OR-fold on their
                // own account once that producer lands — see the type-level
                // doc note above for why this does NOT parallel the earlier
                // `Content { keyword: Before | After }` arm). Spec
                // confirmation is still pending on this classification.
            }
            // Non-exhaustive catch-all: any new ContentComponent variant
            // should be checked against spec before being allowed to fall
            // through here.
            _ => {}
        }
    }
    DynamicFlags {
        has_counter,
        has_string,
        has_target,
        has_content_variant,
    }
}

// ── register-site walker (producer) ─────────────

/// Document-order arena walk that registers every `position: running(name)`
/// element into a fresh [`RunningTemplateStore`] — the "register-site
/// walker" design §7.3 calls out as the producer half of the tier-1 cache.
///
/// Walks `doc` from its root in document order (iterative DFS, same
/// reverse-push-children shape as [`crate::layout::find_body`], so pool
/// order == document order, per [`RunningTemplateStore::register`]'s caller
/// invariant). For every in-document [`NodeKind::Element`] whose
/// `cascade.computed[idx].running_templates` is non-empty (the cascade-time
/// seed raikiri-style emits, design §7.0 static side), builds a
/// [`ParsedRunningTemplate`] via [`collect_running_template`] and calls
/// [`RunningTemplateStore::register`].
///
/// **Nested `position: running(name)` elements are not pruned.** A running
/// element nested inside another running element's subtree is visited (and
/// registered) both as part of the outer template's [`CascadeSubset`] (its
/// `computed_styles` include the nested subtree — DOM structure is
/// unaffected by `running()`, same as `display: none`) and independently as
/// its own top-level registration, uniformly like every other running
/// element. See [`collect_running_template`]'s doc for why the outer
/// template's `directives` does NOT also carry a `RegisterRunning` entry for
/// the nested seed.
///
/// # Precondition
///
/// `doc`'s `IS_IN_DOCUMENT` flags must be up to date
/// (see [`Document::mark_in_document_flags`]) — the same precondition
/// [`raikiri_style::cascade()`] itself carries, since this walker is meant to
/// run against the very `cascade` output produced from `doc`.
#[allow(
    dead_code,
    reason = "Register-site walker already landed; no \
              production driver calls it yet (that's the per-page \
              PageStream / paint integration, a later wall/dom-paint task — \
              see this function's Non-goals section). Exercised via \
              unit tests until then, same status the pieces it wires \
              together previously carried individually."
)]
pub(crate) fn build_running_template_store(
    doc: &Document,
    cascade: &CascadeResult,
) -> RunningTemplateStore {
    let mut store = RunningTemplateStore::default();
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(idx) = stack.pop() {
        let node = &doc.nodes[idx];
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element
            && let Some(rt) = cascade
                .computed
                .get(idx)
                .and_then(|cv| cv.running_templates.first())
        {
            let name = Symbol::new(rt.name.clone());
            let template = collect_running_template(doc, cascade, idx);
            store.register(name, template);
        }
        // Reverse-push children so the stack pops them in document order
        // (same shape as `crate::layout::find_body`).
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    store
}

/// Build a [`ParsedRunningTemplate`] for the subtree rooted at
/// `subtree_root` — the per-template half of
/// [`build_running_template_store`]'s walk.
///
/// Walks the subtree in document order (same DFS shape as the caller),
/// collecting for every in-document node:
/// - its [`raikiri_style::ComputedValues`] into [`CascadeSubset::styles`]
///   (walk order, per that field's doc);
/// - `counter-reset` / `counter-increment` / `counter-set` entries as
///   [`GcpmDirective::CounterReset`] / [`GcpmDirective::CounterIncrement`] /
///   [`GcpmDirective::CounterSet`] (one directive per `(name, value)` pair),
///   pushed in that CSS Lists 3 §4 processing order (not property
///   declaration order) — see the loop's own comment;
/// - `string-set` entries as [`GcpmDirective::StringSet`], via
///   [`resolve_string_set_component`] (per-item `attr()`/bare `content()`
///   resolution against this element, the one point in the pipeline that
///   still has DOM access — see that function's doc) followed by
///   [`convert_string_set_source`] — see both functions' docs for the
///   skip-whole-entry-on-failure policy.
/// - the node's `content` list, folded (OR) into the template's aggregate
///   [`DynamicFlags`] via [`detect_dynamic_flags`] — the "real invocation
///   site" this task adds (previously exercised only by unit tests calling
///   `detect_dynamic_flags` directly).
///
/// Does NOT emit `RegisterRunning` for a nested `position: running(name)`
/// seed found while walking the subtree — the nested element is registered
/// independently by [`build_running_template_store`]'s own top-level walk;
/// see [`ParsedRunningTemplate`]'s type-level `directives` field note for
/// why this walker leaves that hedge unresolved (fail-closed, 原則 3) rather
/// than guess a directive shape nothing downstream consumes yet. Does NOT
/// emit `RegisterTarget` either — out of this task's scope (see the
/// module-level "Divergence from `raikiri_traits::TargetRegistry`'s
/// register policy" note).
#[allow(
    dead_code,
    reason = "Helper for build_running_template_store; \
              same not-yet-production-driven status."
)]
fn collect_running_template(
    doc: &Document,
    cascade: &CascadeResult,
    subtree_root: usize,
) -> ParsedRunningTemplate {
    let mut styles = Vec::new();
    let mut directives = Vec::new();
    let mut dynamic_flags = DynamicFlags::default();

    let mut stack: Vec<usize> = vec![subtree_root];
    while let Some(idx) = stack.pop() {
        let node = &doc.nodes[idx];
        if !node.is_in_document() {
            continue;
        }
        if let Some(cv) = cascade.computed.get(idx) {
            styles.push(cv.clone());

            // Pushed in CSS Lists 3 §4 "Automatic Numbering With Counters"
            // processing order (reset → increment → set —
            // <https://www.w3.org/TR/css-lists-3/#auto-numbering>, §4.2's
            // note that counter-set is applied after counter-increment).
            // No consumer walks this Vec yet (see this function's doc), but
            // the future Phase B walk is expected to
            // apply it front-to-back rather than re-sort by directive kind,
            // so getting push order right now avoids baking in a
            // same-element `counter-reset: c 0; counter-increment: c 1`
            // ordering bug that nothing here would catch.
            for (name, value) in cv.counter_reset.iter() {
                directives.push(GcpmDirective::CounterReset {
                    name: Symbol::new(name.clone()),
                    value: *value,
                });
            }
            for (name, delta) in cv.counter_increment.iter() {
                directives.push(GcpmDirective::CounterIncrement {
                    name: Symbol::new(name.clone()),
                    delta: *delta,
                });
            }
            for (name, value) in cv.counter_set.iter() {
                directives.push(GcpmDirective::CounterSet {
                    name: Symbol::new(name.clone()),
                    value: *value,
                });
            }
            for (name, content_list) in cv.string_set.iter() {
                // Resolve attr()/bare content() against `idx` (this
                // element) before the content-list ever leaves raikiri-dom
                // — see resolve_string_set_component's doc for why this is
                // the only point in the pipeline with DOM element access.
                let resolved: Option<Vec<ContentComponent>> = content_list
                    .iter()
                    .cloned()
                    .map(|component| resolve_string_set_component(doc, idx, component))
                    .collect();
                if let Some(resolved) = resolved
                    && let Some(source) = convert_string_set_source(&resolved)
                {
                    directives.push(GcpmDirective::StringSet {
                        name: Symbol::new(name.clone()),
                        source,
                    });
                }
                // Resolution failure (content(before)/content(after)/
                // content(first-letter), still unresolvable here — see
                // resolve_string_set_component's doc) or conversion failure
                // → skip this string-set entry entirely. Note attr() on a
                // missing attribute is NOT a resolution failure — it
                // resolves to Literal("") (see resolve_string_set_component's
                // Attr-arm doc), same as the present-but-empty case.
            }

            let node_flags = detect_dynamic_flags(&cv.content);
            dynamic_flags.has_counter |= node_flags.has_counter;
            dynamic_flags.has_string |= node_flags.has_string;
            dynamic_flags.has_target |= node_flags.has_target;
            dynamic_flags.has_content_variant |= node_flags.has_content_variant;
            // cov:ignore: the None arm of this defensive `.get(idx)` (unreached
            // here — every idx on the stack comes from `doc`'s own `children`
            // links, and `raikiri_style::cascade`'s own contract is
            // `computed.len() == doc.node_count()`, so `idx` is always in
            // bounds for the `cascade` this walker is called with) is
            // impossible to hit without passing a `cascade` computed against a
            // different `Document`, which isn't a real call pattern for this
            // crate-private helper and isn't reachable from outside the crate
            // to construct adversarially (`CascadeResult` is `#[non_exhaustive]`
            // cross-crate).
        }

        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    ParsedRunningTemplate {
        subtree_root: NodeId::new(subtree_root as u64),
        computed_styles: Arc::new(CascadeSubset { styles }),
        directives,
        dynamic_flags,
    }
}

/// Convert a resolved `string-set` content-list
/// ([`raikiri_style::ComputedValues::string_set`])
/// into a [`ContentSource`], or `None` if any component fails
/// [`ContentValueItem`]'s `TryFrom<ContentComponent>`.
///
/// **Whole-entry skip, not partial-item truncation** — if any component in
/// `content_list` fails to convert, the entire `string-set` entry is
/// dropped rather than emitting a `ContentSource` missing just that item
/// (a truncated content-list would silently change the named string's
/// resolved value instead of omitting it, which is worse).
///
/// **Currently unreachable via a real `string-set:` CSS declaration** — the
/// `GcpmStringSet` parser mode
/// ([`raikiri_style::property`]'s `parse_content_function`) already rejects
/// `target-counter()` / `target-counters()` / `target-text()` / `string()` /
/// `leader()` / `<image>` / `contents` / `<quote>` at parse time, and every
/// component `GcpmStringSet` mode *does* accept (`Literal`, `Counter`,
/// `Counters`, `Attr`, `Content`) converts via `TryFrom` unconditionally (no
/// URL, no fallible mapping). So today this function's `None` branch is
/// defense-in-depth, not a reachable production path — kept for a future
/// `ContentTextKeyword` variant this crate hasn't mirrored yet, or a future
/// relaxation of the `GcpmStringSet` grammar that lets a URL-bearing
/// component through (at which point the common failure would be
/// `Url::parse` rejecting a relative `target-counter(url(#frag), ...)`
/// reference — the `url` crate requires an absolute base for every URL,
/// including fragment-only ones — a wall/traits base-URL-resolution gap,
/// not something this task fixes). Pinned directly (bypassing the CSS
/// parser) by
/// `convert_string_set_source_skips_entry_on_url_conversion_failure`.
#[allow(
    dead_code,
    reason = "Helper for collect_running_template; \
              same not-yet-production-driven status."
)]
fn convert_string_set_source(content_list: &[ContentComponent]) -> Option<ContentSource> {
    content_list
        .iter()
        .cloned()
        .map(ContentValueItem::try_from)
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .map(ContentSource::new)
}

/// Resolve an `attr()` / bare `content()` item from a `string-set`
/// content-list against the DOM element the declaration is on (`doc.nodes
/// [idx]`) — the one point in the pipeline that still has that element in
/// hand. `PageContext::apply_directive`'s `resolve_content_source`
/// (`raikiri-traits`) cannot reach it: by the time a `string-set` value
/// becomes a flat [`GcpmDirective::StringSet`], the originating element is
/// gone (see that function's doc, "DOM element access ... this
/// directive-only resolution path structurally cannot have"). Resolving
/// here, before [`convert_string_set_source`] ever builds the
/// [`ContentSource`], turns these items into plain
/// [`ContentComponent::Literal`]s so the later resolve pass treats them
/// like any other literal text — no `raikiri-traits` change needed.
///
/// **Resolving here (collection time) is equivalent to CSS GCPM 3
/// §1.1.1's "assigned at the point when the content box of the element is
/// first created"**, unlike `counter()`/`counters()`, which
/// [`raikiri_dom::gcpm::StringSnapshot`]'s doc explains must be frozen at
/// *directive-apply* time because their value drifts as later siblings
/// mutate shared counter state. An attribute value and an element's own
/// descendant text don't have that per-page drift — they're fixed
/// per-element, which is exactly why this module's own
/// [`detect_dynamic_flags`] already classifies `Attr` and `content(text)`
/// as static (no per-page dependency), so resolving them once, here,
/// carries no staleness risk the later apply-time resolve would have
/// avoided.
///
/// - [`ContentComponent::Attr`] (CSS Values and Units 5 §7.7.1
///   <https://www.w3.org/TR/css-values-5/#attr-notation>): if the element
///   carries the named attribute — even `""` — the attribute's value
///   substitutes verbatim. If the element does not carry the attribute,
///   this resolves to `Literal("")` too, not a dropped assignment — §7.7.1's
///   own prose (immediately preceding "To resolve an attr() function"):
///   "If the `<syntax>` argument is omitted, the fallback defaults to the
///   empty string if omitted; otherwise, it defaults to the
///   guaranteed-invalid value if omitted." raikiri's `attr()` parser only
///   accepts the bare 1-argument form (no `<syntax>`, no explicit fallback —
///   see `raikiri_style::property::parse_attr_fn`'s doc, "type / fallback
///   ... defer"), which is exactly the "`<syntax>` omitted" branch, so the
///   applicable default is the empty string, not the guaranteed-invalid
///   value. (The algorithm's own step 4 states the guaranteed-invalid
///   default unconditionally, without threading the `<syntax>`-presence
///   branch the prose describes — read literally in isolation that step
///   would make the prose's "if `<syntax>` is omitted" clause vacuous for
///   every input, which is more likely a drafting gap in an active Working
///   Draft than the intended rule. GCPM 3's own `<content-list>` grammar for
///   `string-set` cites the older untyped-only `[CSS-VALUES-3]` `attr()`
///   for its `<attr()>` term, not this typed Values 5 form, and legacy
///   `content: attr(x)` — CSS 2.1 §12.2 — has resolved a missing attribute
///   to the empty string since the property existed, both consistent with
///   treating a missing attribute as "empty string, assignment still
///   occurs" here.) This converges with the present-but-empty-attribute
///   case above on the same `Literal("")` outcome, which matters because
///   CSS GCPM 3 §1.1.1 fixes *that an assignment occurs* at content-box
///   creation independent of what it resolves to, and `string()`'s
///   `first-except` keyword (§1.1.2) is defined by whether an assignment
///   occurred at all — not by what it resolved to.
/// - [`ContentComponent::Content`] with the default/`text` keyword (CSS
///   GCPM 3 §1.1.1.1 <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>,
///   "The string value of the element, determined as if `white-space:
///   normal` had been set"): resolved via [`element_text_string_value`].
/// - [`ContentComponent::Content`] with `before` / `after` / `first-letter`:
///   left untouched, i.e. still unresolvable past this point (same
///   fail-closed skip as before this fix) — `before`/`after` need the
///   target pseudo-element's own content-list, and `first-letter` needs a
///   `::first-letter` segmentation pass; neither is available to this
///   per-node collection walk.
/// - every other variant passes through unchanged.
#[allow(
    dead_code,
    reason = "Helper for collect_running_template; \
              same not-yet-production-driven status."
)]
fn resolve_string_set_component(
    doc: &Document,
    idx: usize,
    component: ContentComponent,
) -> Option<ContentComponent> {
    match component {
        ContentComponent::Attr { name } => match &doc.nodes[idx].data {
            NodeData::Element(e) => Some(ContentComponent::Literal(
                e.attributes
                    .iter()
                    .find(|a| a.local == name)
                    .map(|a| a.value.clone())
                    .unwrap_or_default(),
            )),
            _ => None,
        },
        ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        } => Some(ContentComponent::Literal(SmolStr::new(
            element_text_string_value(doc, idx),
        ))),
        other => Some(other),
    }
}

/// Approximate CSS GCPM 3 §1.1.1.1's "the string value of the element,
/// determined as if `white-space: normal` had been set" for `content()` /
/// `content(text)`. The spec gives no normative algorithm for "string
/// value" beyond that sentence, so this concatenates every descendant
/// [`NodeData::Text`] node's character data in document order (skipping any
/// subtree with `IS_IN_DOCUMENT` cleared, same convention as this module's
/// other walks) and collapses runs of **ASCII** whitespace (space / tab /
/// LF / CR / FF) to a single space with both ends trimmed —
/// [`str::split_ascii_whitespace`] already implements exactly that
/// collapsing. Deliberately not [`str::split_whitespace`] (Unicode
/// `White_Space`, which would also swallow U+00A0 NO-BREAK SPACE): CSS
/// Text 3 §4.1 <https://www.w3.org/TR/css-text-3/#white-space-processing>
/// scopes `white-space: normal` collapsing to "spaces (U+0020), tabs
/// (U+0009), and segment breaks", explicitly carving out no-break space —
/// collapsing it here would be wrong, not just imprecise.
///
/// Does not include generated content (`::before`/`::after`) — that's
/// `content(before)` / `content(after)`'s job, out of scope here (see
/// [`resolve_string_set_component`]'s doc).
#[allow(
    dead_code,
    reason = "Helper for resolve_string_set_component; \
              same not-yet-production-driven status."
)]
fn element_text_string_value(doc: &Document, idx: usize) -> String {
    let mut raw = String::new();
    collect_descendant_text(doc, idx, &mut raw);
    raw.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// DFS accumulator for [`element_text_string_value`] — appends every
/// descendant [`NodeData::Text`] node's raw character data (pre-whitespace-
/// collapse) to `out`, in document order.
#[allow(
    dead_code,
    reason = "Helper for element_text_string_value; \
              same not-yet-production-driven status."
)]
fn collect_descendant_text(doc: &Document, idx: usize, out: &mut String) {
    let node = &doc.nodes[idx];
    if !node.is_in_document() {
        return;
    }
    if let NodeData::Text(t) = &node.data {
        out.push_str(t.text_content.as_str());
    }
    for &child in &node.children {
        collect_descendant_text(doc, child, out);
    }
}

/// Per-page margin box geometry — the input to [`layout_running_template`].
///
/// Design doc §7.3 line 2008: `(page_name, effective_width, effective_height,
/// page_context)` is confirmed by PageStream from the §9.1 page-name
/// transitions. This struct is the dom-internal shape; PageContext plumbing
/// itself is a paint/dom-paint concern and stays out of this task's scope
/// (wall/dom-paint would trigger if this struct grew into a paint-observable
/// contract).
#[derive(Debug, Clone, PartialEq)]
#[allow(
    dead_code,
    reason = "Consumed by layout_running_template; producer is the PageStream \
              driver landing with a future paint integration."
)]
pub(crate) struct MarginBoxGeometry {
    /// The `@page` selector name that resolved to this page
    /// (e.g. `@page chapter`). `None` = the default `@page` block.
    pub(crate) page_name: Option<Symbol>,
    /// Effective width of the margin box (CSS px).
    pub(crate) width: f32,
    /// Effective height of the margin box (CSS px).
    pub(crate) height: f32,
}

/// Result of a per-page running-template layout — the shape [`MarginBoxFragment`]
/// (paint-side) will eventually consume.
///
/// **Deliberate stub** — the paint-observable `MarginBoxFragment` lives on
/// the paint side, and populating it now would cross wall/dom-paint. This
/// dom-internal placeholder carries the id and geometry the paint side
/// already knows how to fetch; the actual fragment tree lands with the paint
/// integration.
#[derive(Debug, Clone, PartialEq)]
#[allow(
    dead_code,
    reason = "Return shape of layout_running_template; consumer landing with \
              the paint integration."
)]
pub(crate) struct MarginBoxLayoutResult {
    /// The template that was laid out.
    pub(crate) template_id: RunningTemplateId,
    /// The geometry it was laid out against (for cache-key parity if a
    /// future tier-2 static-template cache lands).
    pub(crate) geometry: MarginBoxGeometry,
    /// Whether the source template was `dynamic_flags.is_fully_static()`.
    /// Future layout-result cache decision key — captured here so the
    /// eventual cache doesn't need to re-open the [`ParsedRunningTemplate`].
    pub(crate) source_was_static: bool,
}

/// Per-page re-layout entry point (design §7.3 lines 2007-2012).
///
/// **Always per-page** — the current flow is "no layout-result cache, always
/// re-layout" (§7.3 line 1979, 2000, 2014-2016). Fully-static templates
/// (`parsed.dynamic_flags.is_fully_static()`) are annotated on the return so
/// a future caller can decide whether to memoize; this function itself does
/// no caching.
///
/// **Stub for this task** — the return type is
/// [`MarginBoxLayoutResult`] rather than a paint fragment tree so the shape
/// stays dom-internal (see [`MarginBoxLayoutResult`] doc). Wiring to the
/// actual taffy/parley layout of the template subtree lands with the paint
/// integration.
#[allow(
    dead_code,
    reason = "Called from the per-page driver landing with the paint \
              integration (a later, separate wall/dom-paint task)."
)]
pub(crate) fn layout_running_template(
    template_id: RunningTemplateId,
    parsed: &ParsedRunningTemplate,
    geometry: MarginBoxGeometry,
) -> MarginBoxLayoutResult {
    // Per-page re-layout is unconditional today; when the tier-2 cache
    // lands, its key will need at least (template_id, page_name, effective
    // margin-box size). The source_was_static bit is captured so the cache
    // can gate on it without re-touching `parsed`.
    let source_was_static = parsed.dynamic_flags.is_fully_static();
    MarginBoxLayoutResult {
        template_id,
        geometry,
        source_was_static,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use raikiri_style::property::{
        ContentPart, ContentTextKeyword, CounterStyle, LeaderType, QuoteKeyword, StringFetchMode,
    };
    use raikiri_style::{build_rule_tree, cascade};
    use smol_str::SmolStr;
    use taffy::Style;

    // ── Canonical-shape pins (design §7.3) ─────────────────────────

    #[test]
    fn running_template_store_default_is_empty() {
        let store = RunningTemplateStore::default();
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn parsed_running_template_shape_matches_design_7_3() {
        // Canonical-shape pin (design §7.3 lines 1982-1987):
        //   subtree_root: NodeId
        //   computed_styles: Arc<CascadeSubset>
        //   directives: Vec<GcpmDirective>
        //   dynamic_flags: DynamicFlags
        // If any field name/type drifts, coordinator should reconcile before
        // landing the follow-ups.
        let parsed = ParsedRunningTemplate {
            subtree_root: NodeId::new(42),
            computed_styles: Arc::new(CascadeSubset::default()),
            directives: Vec::new(),
            dynamic_flags: DynamicFlags::default(),
        };
        assert_eq!(parsed.subtree_root, NodeId::new(42));
        assert!(parsed.computed_styles.styles.is_empty());
        // Default empty (populated by the later register-site walker; the
        // enum shape has already landed).
        assert!(parsed.directives.is_empty());
        assert_eq!(parsed.dynamic_flags, DynamicFlags::default());
    }

    #[test]
    fn dynamic_flags_default_is_all_false_and_static() {
        let f = DynamicFlags::default();
        assert!(!f.has_counter);
        assert!(!f.has_string);
        assert!(!f.has_target);
        assert!(!f.has_content_variant);
        assert!(f.is_fully_static());
    }

    #[test]
    fn dynamic_flags_is_fully_static_flips_on_any_axis() {
        // Regression pin: is_fully_static must be `all four false`, not just
        // "any" false — sloppy `!self.has_X || ...` would let a single-axis
        // template mis-qualify for a future static cache.
        assert!(DynamicFlags::default().is_fully_static());
        assert!(
            !DynamicFlags {
                has_counter: true,
                ..DynamicFlags::default()
            }
            .is_fully_static()
        );
        assert!(
            !DynamicFlags {
                has_string: true,
                ..DynamicFlags::default()
            }
            .is_fully_static()
        );
        assert!(
            !DynamicFlags {
                has_target: true,
                ..DynamicFlags::default()
            }
            .is_fully_static()
        );
        assert!(
            !DynamicFlags {
                has_content_variant: true,
                ..DynamicFlags::default()
            }
            .is_fully_static()
        );
    }

    // ── Tier-1 cache (register / resolve_element_pool / get) ──────

    fn make_parsed(subtree_root: u64, flags: DynamicFlags) -> ParsedRunningTemplate {
        ParsedRunningTemplate {
            subtree_root: NodeId::new(subtree_root),
            computed_styles: Arc::new(CascadeSubset::default()),
            directives: Vec::new(),
            dynamic_flags: flags,
        }
    }

    #[test]
    fn resolve_element_pool_returns_empty_slice_for_unregistered_name() {
        // GCPM 3 §1.2.2: an unresolvable `element(name, ...)` reference —
        // no element in the pool that satisfies the caller's selector
        // keyword — evaluates to empty content. This store returns &[] for
        // an empty pool; the caller decides the empty fallback (mirrors
        // target.rs's non-fragment-URL empty-string fallback).
        let store = RunningTemplateStore::default();
        assert!(
            store
                .resolve_element_pool(&Symbol::new("header"))
                .is_empty()
        );
    }

    #[test]
    fn register_returns_subtree_root_as_id_and_get_finds_template() {
        // Tier-1 cache hit path via id: register a template, look it up by
        // id, get the same subtree_root back.
        let mut store = RunningTemplateStore::default();
        let id = store.register(
            Symbol::new("header"),
            make_parsed(7, DynamicFlags::default()),
        );
        assert_eq!(id, RunningTemplateId::new(NodeId::new(7)));
        assert_eq!(store.len(), 1);

        let template = store.get(id).expect("just registered");
        assert_eq!(template.subtree_root, NodeId::new(7));
    }

    #[test]
    fn pool_retains_all_same_name_registrations_in_document_order() {
        // CSS GCPM 3 §1.2.2's `element(<name>, [first|start|last|first-except]?)`
        // (https://www.w3.org/TR/css-gcpm-3/#element-syntax) selects per page
        // over the ordered pool (default keyword `first`). Requires retaining
        // the full pool; a name-keyed store with insert-replace semantics
        // would collapse to only the last registered element and foreclose
        // per-page selection under every selector-keyword variant
        // (regression pin against that bug — called out by advisor pre-amend).
        let mut store = RunningTemplateStore::default();
        // Three chapter headers, registered in document order.
        let id_a = store.register(
            Symbol::new("chapter-header"),
            make_parsed(11, DynamicFlags::default()),
        );
        let id_b = store.register(
            Symbol::new("chapter-header"),
            make_parsed(22, DynamicFlags::default()),
        );
        let id_c = store.register(
            Symbol::new("chapter-header"),
            make_parsed(33, DynamicFlags::default()),
        );

        // parsed_templates retains all three.
        assert_eq!(store.len(), 3);
        assert_eq!(
            store.get(id_a).map(|p| p.subtree_root),
            Some(NodeId::new(11))
        );
        assert_eq!(
            store.get(id_b).map(|p| p.subtree_root),
            Some(NodeId::new(22))
        );
        assert_eq!(
            store.get(id_c).map(|p| p.subtree_root),
            Some(NodeId::new(33))
        );

        // Pool preserves registration (== document) order.
        let pool = store.resolve_element_pool(&Symbol::new("chapter-header"));
        assert_eq!(pool, &[id_a, id_b, id_c]);
    }

    #[test]
    fn pool_preserves_registration_order() {
        // Explicit regression pin: the register-site walker
        // (build_running_template_store) calls register in document order,
        // and the pool must faithfully preserve that order — CSS GCPM 3
        // §1.2.2 selector-keyword semantics (first/start/last/first-except)
        // all filter over an ordered pool. If a future implementation
        // switches to a HashSet / BTreeSet, this test breaks.
        let mut store = RunningTemplateStore::default();
        let name = Symbol::new("hdr");
        let ids: Vec<_> = (1..=5u64)
            .map(|n| store.register(name.clone(), make_parsed(n * 10, DynamicFlags::default())))
            .collect();
        let pool = store.resolve_element_pool(&name);
        assert_eq!(pool, ids.as_slice());
    }

    #[test]
    fn pool_scopes_by_name_no_cross_leak() {
        // Distinct names ("header" vs "footer") must not collide in the
        // name_pool index. Regression pin against a bug where a shared
        // vec would accumulate ids across names.
        let mut store = RunningTemplateStore::default();
        let hdr = store.register(
            Symbol::new("header"),
            make_parsed(3, DynamicFlags::default()),
        );
        let ftr = store.register(
            Symbol::new("footer"),
            make_parsed(5, DynamicFlags::default()),
        );
        assert_eq!(store.len(), 2);
        assert_eq!(store.resolve_element_pool(&Symbol::new("header")), &[hdr]);
        assert_eq!(store.resolve_element_pool(&Symbol::new("footer")), &[ftr]);
    }

    #[test]
    fn re_register_same_id_replaces_template_and_leaves_pool_unchanged() {
        // Same subtree_root, same name → same RunningTemplateId. Re-
        // registration replaces the parsed template (repeat cascade
        // robustness), does NOT duplicate the id in the name pool.
        let mut store = RunningTemplateStore::default();
        let id_first = store.register(
            Symbol::new("hdr"),
            make_parsed(
                7,
                DynamicFlags {
                    has_counter: true,
                    ..DynamicFlags::default()
                },
            ),
        );
        // Same id (same subtree_root), different flags.
        let id_second = store.register(
            Symbol::new("hdr"),
            make_parsed(
                7,
                DynamicFlags {
                    has_string: true,
                    ..DynamicFlags::default()
                },
            ),
        );
        assert_eq!(id_first, id_second);
        assert_eq!(store.len(), 1, "same id must not duplicate");

        // Pool has one entry, not two.
        let pool = store.resolve_element_pool(&Symbol::new("hdr"));
        assert_eq!(pool, &[id_first]);

        // parsed_templates carries the LATEST template (last-wins per id).
        let latest = store.get(id_first).expect("registered");
        assert!(!latest.dynamic_flags.has_counter);
        assert!(latest.dynamic_flags.has_string);
    }

    #[test]
    fn re_register_same_id_under_different_name_transfers_pool_entry() {
        // Same-id-different-name re-registration must keep the store
        // self-consistent: the id must NOT remain in the previous name's
        // pool (that would leak) and MUST appear in the new name's pool
        // (otherwise resolve_element_pool wouldn't find it under either
        // name). This case is not expected in production (the register-site
        // walker walks each element once, and position: running(<name>) is
        // a single computed value per element), but the transfer keeps the
        // store robust under repeated cascade / unforeseen recursive walks
        // — regression pin against the pre-amend `is_new_id` gate that
        // skipped the pool update and left the pool stale.
        let mut store = RunningTemplateStore::default();
        let id_first = store.register(
            Symbol::new("header"),
            make_parsed(7, DynamicFlags::default()),
        );
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("header")),
            &[id_first]
        );

        // Same subtree_root re-registered under a NEW name.
        let id_second = store.register(
            Symbol::new("footer"),
            make_parsed(7, DynamicFlags::default()),
        );
        assert_eq!(id_first, id_second, "same subtree_root → same id");
        assert_eq!(store.len(), 1, "still one parsed template");

        // The previous name's pool must no longer contain the id — and
        // must be empty (only registration).
        assert!(
            store
                .resolve_element_pool(&Symbol::new("header"))
                .is_empty(),
            "id must be removed from the previous name's pool"
        );

        // The new name's pool must contain the id in the sole slot.
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("footer")),
            &[id_first],
            "id must land in the new name's pool"
        );

        // Third transfer back to the original name (idempotent under
        // repeated cycles).
        store.register(
            Symbol::new("header"),
            make_parsed(7, DynamicFlags::default()),
        );
        assert!(
            store
                .resolve_element_pool(&Symbol::new("footer"))
                .is_empty(),
            "second transfer must drain the footer pool"
        );
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("header")),
            &[id_first]
        );
    }

    #[test]
    fn re_register_transfer_leaves_other_ids_in_prev_pool_intact() {
        // A name pool containing MULTIPLE ids must keep siblings intact when
        // one id transfers away; only the transferring id gets removed. If
        // this invariant broke, transferring id A out of the "header" pool
        // could also drop sibling id B, silently corrupting per-page
        // resolution.
        let mut store = RunningTemplateStore::default();
        let id_a = store.register(
            Symbol::new("header"),
            make_parsed(11, DynamicFlags::default()),
        );
        let id_b = store.register(
            Symbol::new("header"),
            make_parsed(22, DynamicFlags::default()),
        );
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("header")),
            &[id_a, id_b]
        );

        // Transfer id_a to "footer"; id_b must remain in "header".
        store.register(
            Symbol::new("footer"),
            make_parsed(11, DynamicFlags::default()),
        );
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("header")),
            &[id_b],
            "sibling id_b must remain in the header pool after id_a transfer"
        );
        assert_eq!(
            store.resolve_element_pool(&Symbol::new("footer")),
            &[id_a],
            "id_a landed in the footer pool"
        );
    }

    // ── DynamicFlags detection (detect_dynamic_flags) ──────────────

    #[test]
    fn detect_dynamic_flags_empty_content_all_false() {
        let f = detect_dynamic_flags(&[]);
        assert_eq!(f, DynamicFlags::default());
        assert!(f.is_fully_static());
    }

    #[test]
    fn detect_dynamic_flags_literal_and_attr_are_static() {
        let cs = vec![
            ContentComponent::Literal(SmolStr::new("hello")),
            ContentComponent::Attr {
                name: SmolStr::new("href"),
            },
        ];
        let f = detect_dynamic_flags(&cs);
        assert!(
            f.is_fully_static(),
            "literal/attr must not flip any dynamic flag"
        );
    }

    #[test]
    fn detect_dynamic_flags_counter_flips_has_counter() {
        let cs = vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }];
        let f = detect_dynamic_flags(&cs);
        assert!(f.has_counter);
        assert!(!f.has_string && !f.has_target && !f.has_content_variant);
    }

    #[test]
    fn detect_dynamic_flags_counters_also_flips_has_counter() {
        // Both counter() and counters() must flip the same flag.
        let cs = vec![ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: ".".to_owned(),
            style: CounterStyle::Decimal,
        }];
        let f = detect_dynamic_flags(&cs);
        assert!(f.has_counter);
    }

    #[test]
    fn detect_dynamic_flags_string_flips_has_string() {
        let cs = vec![ContentComponent::String {
            name: SmolStr::new("chapter-title"),
            fetch: StringFetchMode::Start,
        }];
        let f = detect_dynamic_flags(&cs);
        assert!(f.has_string);
        assert!(!f.has_counter && !f.has_target && !f.has_content_variant);
    }

    #[test]
    fn detect_dynamic_flags_target_all_three_variants_flip_has_target() {
        // target-counter / target-counters / target-text all flip has_target
        // (one runtime-dependency axis, not three separate ones).
        let counter_cc = vec![ContentComponent::TargetCounter {
            url: "#c".to_owned(),
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }];
        assert!(detect_dynamic_flags(&counter_cc).has_target);

        let counters_cc = vec![ContentComponent::TargetCounters {
            url: "#s".to_owned(),
            name: SmolStr::new("section"),
            separator: ".".to_owned(),
            style: CounterStyle::Decimal,
        }];
        assert!(detect_dynamic_flags(&counters_cc).has_target);

        let text_cc = vec![ContentComponent::TargetText {
            url: "#h".to_owned(),
            part: ContentPart::Content,
        }];
        assert!(detect_dynamic_flags(&text_cc).has_target);
    }

    #[test]
    fn detect_dynamic_flags_content_before_and_after_flip_has_content_variant() {
        // content(before) and content(after) read the ::before/::after pseudo
        // string values, which are themselves built from content-lists that
        // may contain page-dependent bits (counter / string / target-*) —
        // so they transitively depend on per-page state. See the DynamicFlags
        // type-level note.
        for kw in [ContentTextKeyword::Before, ContentTextKeyword::After] {
            let cs = vec![ContentComponent::Content { keyword: kw }];
            let f = detect_dynamic_flags(&cs);
            assert!(
                f.has_content_variant,
                "content({kw:?}) must flip has_content_variant"
            );
            assert!(!f.has_counter && !f.has_string && !f.has_target);
        }
    }

    #[test]
    fn detect_dynamic_flags_content_text_and_first_letter_are_static() {
        // Matches design §7.3 line 1993 "content(before/after) 参照あり":
        // content(text) reads the element's own string value, which is fixed
        // per running element; a future layout cache keyed on template_id
        // naturally misses across distinct elements. content(first-letter)
        // is derived from text and follows the same static classification.
        // Regression pin — previously the arm blanket-flipped for any Content
        // variant (over-mark) and this test's assertion was inverted.
        for kw in [ContentTextKeyword::Text, ContentTextKeyword::FirstLetter] {
            let cs = vec![ContentComponent::Content { keyword: kw }];
            let f = detect_dynamic_flags(&cs);
            assert!(
                !f.has_content_variant,
                "content({kw:?}) must NOT flip has_content_variant"
            );
            assert!(
                f.is_fully_static(),
                "content({kw:?}) alone leaves template static"
            );
        }
    }

    #[test]
    fn detect_dynamic_flags_image_contents_quote_leader_are_static() {
        // The `Image` / `Contents` / `Quote` / `Leader` variants (CSS
        // Content 3 §2.2 / §2.3 / §2.4.2 / §2.5.1) each get an explicit
        // no-op arm now — pin that none of them flip a dynamic flag. Spec
        // confirmation is still pending on this "no dynamic flag"
        // classification; this test pins current behavior, not a
        // spec-confirmed final answer.
        let cs = vec![
            ContentComponent::Image {
                url: "cover.png".to_owned(),
            },
            ContentComponent::Contents,
            ContentComponent::Quote(QuoteKeyword::OpenQuote),
            ContentComponent::Quote(QuoteKeyword::CloseQuote),
            ContentComponent::Quote(QuoteKeyword::NoOpenQuote),
            ContentComponent::Quote(QuoteKeyword::NoCloseQuote),
            ContentComponent::Leader(LeaderType::Dotted),
            ContentComponent::Leader(LeaderType::Solid),
            ContentComponent::Leader(LeaderType::Space),
            ContentComponent::Leader(LeaderType::String(SmolStr::new("~"))),
        ];
        let f = detect_dynamic_flags(&cs);
        assert!(
            f.is_fully_static(),
            "Image/Contents/Quote/Leader must not flip any dynamic flag"
        );
    }

    #[test]
    fn detect_dynamic_flags_composes_multiple_axes() {
        // A template mixing counter + string + target + content-variant flips
        // all four flags in a single scan.
        let cs = vec![
            ContentComponent::Literal(SmolStr::new("Chapter ")),
            ContentComponent::Counter {
                name: SmolStr::new("chapter"),
                style: CounterStyle::Decimal,
            },
            ContentComponent::Literal(SmolStr::new(" — ")),
            ContentComponent::String {
                name: SmolStr::new("chapter-title"),
                fetch: StringFetchMode::Start,
            },
            ContentComponent::Literal(SmolStr::new(" (see p. ")),
            ContentComponent::TargetCounter {
                url: "#refpage".to_owned(),
                name: SmolStr::new("page"),
                style: CounterStyle::Decimal,
            },
            ContentComponent::Literal(SmolStr::new(", ")),
            // Use Before (not Text) so this composite covers the
            // has_content_variant axis — content(text) is deliberately
            // static per the refined semantics.
            ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            },
            ContentComponent::Literal(SmolStr::new(")")),
        ];
        let f = detect_dynamic_flags(&cs);
        assert!(f.has_counter);
        assert!(f.has_string);
        assert!(f.has_target);
        assert!(f.has_content_variant);
        assert!(!f.is_fully_static());
    }

    // ── register-site walker (build_running_template_store) ────────

    #[test]
    fn build_running_template_store_registers_element_with_running_position() {
        let mut doc = Document::new();
        let header = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        assert_eq!(store.len(), 1);

        let pool = store.resolve_element_pool(&Symbol::new("header"));
        assert_eq!(pool.len(), 1);
        let template = store.get(pool[0]).expect("registered");
        assert_eq!(template.subtree_root, NodeId::new(header as u64));
    }

    #[test]
    fn build_running_template_store_ignores_elements_without_running_position() {
        let mut doc = Document::new();
        doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        doc.append_element(Some(0), "p", Style::default(), Some("color: red"));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        assert_eq!(store.len(), 0, "no position: running() element present");
    }

    #[test]
    fn build_running_template_store_and_collect_running_template_skip_not_in_document_nodes() {
        // Comment nodes default IS_IN_DOCUMENT=true at construction (see
        // Node::new_comment's doc) but get explicitly cleared by
        // Document::mark_in_document_flags's DFS. Both this walker's
        // top-level register-site walk and collect_running_template's own
        // per-template subtree walk must skip such nodes via their
        // is_in_document() gate, same as every other traversal in this
        // crate (cascade / paint / layout — see layout::find_body's doc).
        // A comment nested inside the running(header) subtree exercises
        // both walks' gates in one pass: the outer walk visits it while
        // looking for further running() elements past this subtree, and
        // collect_running_template visits it while building this
        // subtree's own CascadeSubset.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_comment(Some(root), "note");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        assert_eq!(pool.len(), 1, "the comment must not itself register");
        let template = store.get(pool[0]).expect("registered");

        // Only the running(header) root itself is in-document — the
        // comment must not contribute a ComputedValues entry to the
        // subtree's CascadeSubset.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            template.computed_styles.styles.len(),
            1,
            "the comment child must be skipped by collect_running_template's \
             own walk, not counted alongside the root"
        );
    }

    #[test]
    fn build_running_template_store_registers_siblings_in_document_order() {
        // Regression pin referenced by the module-level "Registration order
        // == document order" note: three sibling running(hdr) elements must
        // land in the pool in document order.
        let mut doc = Document::new();
        let a = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(hdr)"),
        );
        let b = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(hdr)"),
        );
        let c = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(hdr)"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        assert_eq!(store.len(), 3);
        let pool = store.resolve_element_pool(&Symbol::new("hdr"));
        let subtree_roots: Vec<NodeId> = pool
            .iter()
            .map(|id| store.get(*id).expect("registered").subtree_root)
            .collect();
        assert_eq!(
            subtree_roots,
            vec![
                NodeId::new(a as u64),
                NodeId::new(b as u64),
                NodeId::new(c as u64),
            ]
        );
    }

    #[test]
    fn build_running_template_store_registers_nested_running_elements_independently() {
        // Nested position: running() elements are not pruned — both the
        // outer and inner element register as their own top-level template,
        // and the outer template's directives must NOT carry a
        // RegisterRunning entry for the nested seed (see
        // collect_running_template's doc comment).
        let mut doc = Document::new();
        let outer = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        let inner = doc.append_element(
            Some(outer),
            "div",
            Style::default(),
            Some("position: running(footer)"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            store.len(),
            2,
            "outer and inner both registered independently"
        );

        let header_pool = store.resolve_element_pool(&Symbol::new("header"));
        let header = store.get(header_pool[0]).expect("registered");
        assert_eq!(header.subtree_root, NodeId::new(outer as u64));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !header
                .directives
                .iter()
                .any(|d| matches!(d, GcpmDirective::RegisterRunning { .. })),
            "the nested running(footer) seed must not surface as a \
             RegisterRunning directive on the outer template"
        );

        let footer_pool = store.resolve_element_pool(&Symbol::new("footer"));
        let footer = store.get(footer_pool[0]).expect("registered");
        assert_eq!(footer.subtree_root, NodeId::new(inner as u64));
    }

    #[test]
    fn collect_running_template_collects_computed_styles_in_subtree_walk_order() {
        // Per CascadeSubset::styles's doc: walk order parallel to the DOM
        // walk from subtree_root. Distinguish nodes via distinct font-size
        // markers.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header); font-size: 10px"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("font-size: 20px"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("font-size: 30px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        let sizes: Vec<f32> = template
            .computed_styles
            .styles
            .iter()
            .map(|cv| cv.font_size.px())
            .collect();
        assert_eq!(sizes, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn collect_running_template_includes_text_nodes_in_walk_order() {
        // `styles` is a positional contract ("parallel to the DOM walk from
        // subtree_root") that a future paint-driver consumer will index
        // into alongside the arena — it must not silently drop Text nodes.
        // CascadeResult.computed populates Text node slots too (inherited
        // from the parent), so collect_running_template's kind-agnostic
        // walk (no NodeKind::Element filter, unlike the top-level
        // running(name) detection in build_running_template_store) must
        // carry them through.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header); font-size: 10px"),
        );
        let child = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("font-size: 20px"),
        );
        doc.append_text(child, "hello");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        let sizes: Vec<f32> = template
            .computed_styles
            .styles
            .iter()
            .map(|cv| cv.font_size.px())
            .collect();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            sizes,
            vec![10.0, 20.0, 20.0],
            "root (10px), span (20px), then the text child inheriting the \
             span's 20px — three entries, not two"
        );
    }

    #[test]
    fn collect_running_template_emits_counter_and_string_set_directives() {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("counter-increment: chapter 2"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(r#"string-set: title "hello""#),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(
            template
                .directives
                .contains(&GcpmDirective::CounterIncrement {
                    name: Symbol::new("chapter"),
                    delta: 2,
                })
        );
        assert!(template.directives.contains(&GcpmDirective::StringSet {
            name: Symbol::new("title"),
            source: ContentSource::new(vec![ContentValueItem::Literal("hello".to_owned())]),
        }));
    }

    #[test]
    fn collect_running_template_emits_counter_directives_in_css_lists_3_processing_order() {
        // CSS Lists 3 §4 "Automatic Numbering With Counters"
        // (<https://www.w3.org/TR/css-lists-3/#auto-numbering>): counter
        // values on one element are resolved reset → increment → set (§4.2
        // notes counter-set is applied after counter-increment). Pin push
        // order == that processing order (not property declaration order,
        // and not raikiri-style's ComputedValues field order, which happens
        // to match here but is not itself the authority) — nothing else
        // catches a regression here since no consumer walk exists yet (see
        // collect_running_template's doc).
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("counter-set: c 5; counter-increment: c 1; counter-reset: c 0"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            template.directives,
            vec![
                GcpmDirective::CounterReset {
                    name: Symbol::new("c"),
                    value: 0,
                },
                GcpmDirective::CounterIncrement {
                    name: Symbol::new("c"),
                    delta: 1,
                },
                GcpmDirective::CounterSet {
                    name: Symbol::new("c"),
                    value: 5,
                },
            ],
            "must emit reset, then increment, then set — declaration order \
             in the inline style was set/increment/reset, the opposite"
        );
    }

    #[test]
    fn collect_running_template_folds_dynamic_flags_across_subtree_nodes() {
        // Walker-level cross-node OR-fold pin (module doc's "multiple
        // content lists — one per node in the subtree — are folded via
        // repeated calls" contract): child A contributes has_counter, child
        // B (a different node) contributes has_string. Neither node alone
        // would flip both flags.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("content: counter(chapter)"),
        );
        doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("content: string(title)"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(template.dynamic_flags.has_counter);
        assert!(template.dynamic_flags.has_string);
        assert!(!template.dynamic_flags.has_target);
        assert!(!template.dynamic_flags.has_content_variant);
    }

    #[test]
    fn convert_string_set_source_skips_entry_on_url_conversion_failure() {
        // The GcpmStringSet parser mode (raikiri_style::property's
        // parse_content_function) already rejects target-counter() /
        // target-counters() / target-text() / string() / leader() / <image>
        // / contents / <quote> at parse time, so this failure path is
        // unreachable via a real `string-set:` CSS declaration today — see
        // convert_string_set_source's doc comment. This test bypasses the
        // parser and calls the helper directly to pin the
        // skip-whole-entry policy as defense-in-depth.
        let content = [ContentComponent::TargetCounter {
            url: "#c".to_owned(), // relative URL: Url::parse rejects it
            name: SmolStr::new("page"),
            style: CounterStyle::Decimal,
        }];
        assert!(convert_string_set_source(&content).is_none());
    }

    // ── resolve_string_set_component / element_text_string_value ──────

    #[test]
    fn resolve_string_set_component_resolves_attr_from_element_attribute() {
        // CSS Values and Units 5 §7.7.1: the attribute's value substitutes
        // verbatim when the element carries it.
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        doc.set_element_attributes(
            h1,
            vec![(SmolStr::new("data-title"), SmolStr::new("Intro"))],
        );

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Attr {
                name: SmolStr::new("data-title"),
            },
        );
        assert_eq!(
            resolved,
            Some(ContentComponent::Literal(SmolStr::new("Intro")))
        );
    }

    #[test]
    fn resolve_string_set_component_resolves_attr_to_empty_literal_when_attribute_value_is_empty() {
        // Empty attribute value ("") is a valid empty <string>, distinct
        // from a missing attribute — must resolve to Literal(""), not skip
        // (see resolve_string_set_component's doc).
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        doc.set_element_attributes(h1, vec![(SmolStr::new("data-title"), SmolStr::new(""))]);

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Attr {
                name: SmolStr::new("data-title"),
            },
        );
        assert_eq!(resolved, Some(ContentComponent::Literal(SmolStr::new(""))));
    }

    #[test]
    fn resolve_string_set_component_resolves_attr_to_empty_literal_when_attribute_missing() {
        // CSS Values and Units 5 §7.7.1's prose (preceding "To resolve an
        // attr() function"): fallback defaults to the empty string when
        // <syntax> is omitted, which is raikiri's only supported attr()
        // form (no <syntax>, no explicit fallback). Converges with the
        // present-but-empty-attribute case: Literal(""), not a dropped
        // assignment — see resolve_string_set_component's Attr-arm doc.
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Attr {
                name: SmolStr::new("data-title"),
            },
        );
        assert_eq!(resolved, Some(ContentComponent::Literal(SmolStr::new(""))));
    }

    #[test]
    fn resolve_string_set_component_resolves_bare_content_to_element_text_value() {
        // GCPM 3 §1.1.1.1: content() / content(text) = "the string value of
        // the element, determined as if white-space: normal had been set".
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        doc.append_text(h1, "  Loomings   Ch.  1  ");

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        );
        assert_eq!(
            resolved,
            Some(ContentComponent::Literal(SmolStr::new("Loomings Ch. 1")))
        );
    }

    #[test]
    fn resolve_string_set_component_content_text_preserves_no_break_space() {
        // CSS Text 3 §4.1 scopes white-space:normal collapsing to space /
        // tab / segment-break, explicitly excluding U+00A0 NO-BREAK SPACE —
        // regression pin against using str::split_whitespace (Unicode
        // White_Space, which would wrongly collapse/trim it too).
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        doc.append_text(h1, "A\u{A0}\u{A0}B");

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        );
        assert_eq!(
            resolved,
            Some(ContentComponent::Literal(SmolStr::new("A\u{A0}\u{A0}B")))
        );
    }

    #[test]
    fn resolve_string_set_component_content_text_concatenates_across_descendant_elements() {
        // "String value of the element" reaches through descendant
        // elements, not just direct-child text nodes.
        let mut doc = Document::new();
        let h1 = doc.append_element(Some(0), "h1", Style::default(), None::<&str>);
        let em = doc.append_element(Some(h1), "em", Style::default(), None::<&str>);
        doc.append_text(em, "Moby");
        doc.append_text(h1, " Dick");

        let resolved = resolve_string_set_component(
            &doc,
            h1,
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        );
        assert_eq!(
            resolved,
            Some(ContentComponent::Literal(SmolStr::new("Moby Dick")))
        );
    }

    #[test]
    fn resolve_string_set_component_leaves_content_before_after_first_letter_unresolved() {
        // Resolving these needs the target pseudo-element's own
        // content-list (before/after) or a ::first-letter segmentation
        // pass — neither is available here, so they must pass through
        // unchanged (still unresolvable downstream, same as before this
        // fix), not silently coerced to some placeholder text.
        let doc = Document::new();
        for kw in [
            ContentTextKeyword::Before,
            ContentTextKeyword::After,
            ContentTextKeyword::FirstLetter,
        ] {
            let component = ContentComponent::Content { keyword: kw };
            assert_eq!(
                resolve_string_set_component(&doc, 0, component.clone()),
                Some(component),
                "content({kw:?}) must pass through unchanged"
            );
        }
    }

    #[test]
    fn resolve_string_set_component_passes_through_literal_and_counter_unchanged() {
        let doc = Document::new();
        let literal = ContentComponent::Literal(SmolStr::new("hello"));
        assert_eq!(
            resolve_string_set_component(&doc, 0, literal.clone()),
            Some(literal)
        );
        let counter = ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        };
        assert_eq!(
            resolve_string_set_component(&doc, 0, counter.clone()),
            Some(counter)
        );
    }

    // ── collect_running_template: attr() / content() integration ──────

    #[test]
    fn collect_running_template_resolves_string_set_attr_from_element() {
        // Concrete GCPM running-header idiom from the bug report:
        // <h1 data-title="Intro"> with `string-set: chapter
        // attr(data-title)` must resolve to Literal("Intro"), not be
        // silently dropped.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        let h1 = doc.append_element(
            Some(root),
            "h1",
            Style::default(),
            Some("string-set: chapter attr(data-title)"),
        );
        doc.set_element_attributes(
            h1,
            vec![(SmolStr::new("data-title"), SmolStr::new("Intro"))],
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(template.directives.contains(&GcpmDirective::StringSet {
            name: Symbol::new("chapter"),
            source: ContentSource::new(vec![ContentValueItem::Literal("Intro".to_owned())]),
        }));
    }

    #[test]
    fn collect_running_template_resolves_string_set_bare_content_from_descendant_text() {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        let h1 = doc.append_element(
            Some(root),
            "h1",
            Style::default(),
            Some("string-set: chapter content()"),
        );
        doc.append_text(h1, "  Loomings   Ch.  1  ");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(template.directives.contains(&GcpmDirective::StringSet {
            name: Symbol::new("chapter"),
            source: ContentSource::new(vec![ContentValueItem::Literal(
                "Loomings Ch. 1".to_owned()
            )]),
        }));
    }

    #[test]
    fn collect_running_template_resolves_string_set_entry_to_empty_when_attr_missing() {
        // CSS Values and Units 5 §7.7.1's <syntax>-omitted default is the
        // empty string, not the guaranteed-invalid value — an assignment
        // still occurs (CSS GCPM 3 §1.1.1's "assigned at the point when the
        // content box of the element is first created" applies regardless
        // of what the content-list resolves to), with empty content. See
        // resolve_string_set_component's Attr-arm doc.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_element(
            Some(root),
            "h1",
            Style::default(),
            Some("string-set: chapter attr(data-title)"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(
            template.directives.contains(&GcpmDirective::StringSet {
                name: Symbol::new("chapter"),
                source: ContentSource::new(vec![ContentValueItem::Literal(String::new())]),
            }),
            "attr() on a missing attribute must register the assignment \
             with Literal(\"\"), not drop it"
        );
    }

    #[test]
    fn collect_running_template_still_resolves_string_set_literal_strings() {
        // Regression pin: the plain <string> literal case (unaffected by
        // this fix) must keep working exactly as before.
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some("position: running(header)"),
        );
        doc.append_element(
            Some(root),
            "h1",
            Style::default(),
            Some(r#"string-set: chapter "Chapter One""#),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let store = build_running_template_store(&doc, &cr);
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        let template = store.get(pool[0]).expect("registered");

        assert!(template.directives.contains(&GcpmDirective::StringSet {
            name: Symbol::new("chapter"),
            source: ContentSource::new(vec![ContentValueItem::Literal("Chapter One".to_owned())]),
        }));
    }

    #[test]
    fn resolve_first_returns_pool_head_and_none_for_unregistered_name() {
        let mut store = RunningTemplateStore::default();
        assert!(store.resolve_first(&Symbol::new("header")).is_none());

        let id_a = store.register(
            Symbol::new("header"),
            make_parsed(11, DynamicFlags::default()),
        );
        store.register(
            Symbol::new("header"),
            make_parsed(22, DynamicFlags::default()),
        );

        let first = store
            .resolve_first(&Symbol::new("header"))
            .expect("header has registrations");
        assert_eq!(first.subtree_root, NodeId::new(11));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            RunningTemplateId::new(first.subtree_root),
            id_a,
            "resolve_first must return the first-registered (document-first) template"
        );
    }

    // ── layout_running_template (per-page re-layout stub) ──────────

    #[test]
    fn layout_running_template_returns_geometry_and_id_verbatim() {
        // Shape pin: the return threads through the input id/geometry unchanged.
        // Real per-page layout wiring lands with the paint integration; the
        // test verifies the invocation shape the design flow
        // (§7.3 lines 2007-2012) calls out.
        let parsed = make_parsed(11, DynamicFlags::default());
        let id = RunningTemplateId::new(NodeId::new(11));
        let geom = MarginBoxGeometry {
            page_name: Some(Symbol::new("chapter")),
            width: 500.0,
            height: 30.0,
        };
        let result = layout_running_template(id, &parsed, geom.clone());
        assert_eq!(result.template_id, id);
        assert_eq!(result.geometry, geom);
    }

    #[test]
    fn layout_running_template_reports_source_was_static_flag() {
        // Static template → source_was_static = true. Future layout-result
        // cache decision key.
        let static_parsed = make_parsed(11, DynamicFlags::default());
        let geom = MarginBoxGeometry {
            page_name: None,
            width: 500.0,
            height: 30.0,
        };
        let r = layout_running_template(
            RunningTemplateId::new(NodeId::new(11)),
            &static_parsed,
            geom,
        );
        assert!(r.source_was_static);
    }

    #[test]
    fn layout_running_template_reports_source_was_static_false_for_dynamic() {
        // Any dynamic axis → source_was_static = false. Regression pin
        // against a mis-fold in is_fully_static (a broken || / && would let a
        // dynamic template mis-qualify for a future static cache).
        let flags = DynamicFlags {
            has_counter: true,
            ..DynamicFlags::default()
        };
        let dyn_parsed = make_parsed(13, flags);
        let geom = MarginBoxGeometry {
            page_name: None,
            width: 500.0,
            height: 30.0,
        };
        let r = layout_running_template(RunningTemplateId::new(NodeId::new(13)), &dyn_parsed, geom);
        assert!(!r.source_was_static);
    }

    #[test]
    fn layout_running_template_does_not_mutate_store_or_parsed() {
        // §7.3 line 1979: layout 結果はキャッシュしない — per-page re-layout
        // must not stash anything into the store or mutate the parsed
        // template (would leak into a later page's re-layout).
        let mut store = RunningTemplateStore::default();
        let id = store.register(
            Symbol::new("header"),
            make_parsed(7, DynamicFlags::default()),
        );

        let parsed = store.get(id).expect("registered");
        let subtree_root_before = parsed.subtree_root;
        let dyn_flags_before = parsed.dynamic_flags;

        let _ = layout_running_template(
            id,
            parsed,
            MarginBoxGeometry {
                page_name: None,
                width: 500.0,
                height: 30.0,
            },
        );

        let parsed_after = store.get(id).expect("still registered");
        assert_eq!(parsed_after.subtree_root, subtree_root_before);
        assert_eq!(parsed_after.dynamic_flags, dyn_flags_before);
        assert_eq!(store.len(), 1);

        // Pool also intact.
        let pool = store.resolve_element_pool(&Symbol::new("header"));
        assert_eq!(pool, &[id]);
    }
}
