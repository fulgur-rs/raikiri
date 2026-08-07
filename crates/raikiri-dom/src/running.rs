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
//! `parsed_templates` (this module); tier-2 is a post-M8 layout-result cache
//! for fully-static templates (`dynamic_flags = all false`) and is
//! **explicitly deferred** (§7.3 lines 2014-2016 "M1〜M8 は常に re-layout").
//! Per-page re-layout via [`layout_running_template`] is the M1〜M8 flow
//! (§7.3 lines 2007-2012).
//!
//! **Divergence from canonical shape** — same "pub(crate) local until
//! traits reconciliation decision" convention that the pre-bsi
//! `crate::target` (removed) module previously held (that reconciliation has
//! since landed via raikiri-spike-bsi Option C — the TargetRegistry /
//! TargetInfo / ResolveOutcome / PendingResolution / resolve_content_component
//! API now lives at [`raikiri_traits::TargetRegistry`] and friends). This module
//! currently writes `pub(crate)` and does NOT re-export through the crate
//! root; a later reconciliation task will decide whether any of these types
//! need to cross wall/traits or wall/dom-paint (bd raikiri-spike-96u.4 has
//! since landed the [`raikiri_traits::GcpmDirective`] variant populate; a
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
//! PageStream / paint driver side (bd raikiri-spike-96u.4 territory).
//!
//! **`CascadeSubset` local definition** — the design doc names
//! `computed_styles: Arc<CascadeSubset>` but no such type exists in the
//! workspace today. Rather than reach into raikiri-style for a shape that
//! doesn't fit ([`raikiri_style::computed::ComputedValues`] is per-node,
//! `CascadeSubset` is a per-subtree collection) or reach into raikiri-traits
//! to promote a placeholder (wall/traits crossing), this module defines
//! [`CascadeSubset`] as a `pub(crate)` local. This is the same "carry a
//! concrete local type until a cross-crate reconciliation is scheduled"
//! shape that [`raikiri_traits::TargetInfo`] previously held pre-bsi —
//! that reconciliation has since landed (raikiri-spike-bsi Option C
//! wall/traits merge), CascadeSubset awaits a similar promotion.
//!
//! **`ContentComponent::Element { name }` upstream gap** — the css-engine
//! side of `content: element(name)` (parsing the `element(<name>)`
//! functional-notation into a `ContentComponent::Element` variant on
//! [`raikiri_style::property::ContentComponent`]) does NOT exist today; the
//! enum has no `Element` arm. Adding it is a css-engine concern
//! (wall/css-engine, not this task's walls). This module therefore lands the
//! **dom-side deliverable** — name→pool→[`ParsedRunningTemplate`] lookup —
//! which is the actual "element(name) resolve" once the caller has extracted
//! the name from wherever. When the css-engine adds the variant, a driver
//! similar to [`raikiri_traits::resolve_content_component`] can wire
//! ContentComponent → `resolve_element_pool` in a single call. Tracked as
//! bd raikiri-spike-6z0 (filed by this task, blocked on css-engine sprint).
//!
//! **Registration order == document order** — the store assumes the caller
//! invokes [`RunningTemplateStore::register`] in DOM tree order. The
//! register-site walker landing with bd raikiri-spike-96u.4 will walk the
//! arena in document order, so this assumption holds automatically; if it is
//! ever violated, per-page pool selection (any selector-keyword variant)
//! would emit the wrong element and visible layout would drift. Regression
//! pin: the `pool_preserves_registration_order` unit test.
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

use raikiri_style::property::{ContentComponent, ContentTextKeyword};
use raikiri_traits::{GcpmDirective, NodeId, RunningTemplateId, Symbol};

// `RunningTemplateId` is the shared identifier from raikiri-traits
// (design §7.0 line 1904 "shared types → raikiri-traits"). Landed by bd
// raikiri-spike-96u.4 together with the `GcpmDirective` populate — the
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
#[allow(
    dead_code,
    reason = "Populated by the register-site walker landing with bd \
              raikiri-spike-96u.4 (M6 directive-apply pass); exercised via \
              unit tests until then."
)]
pub(crate) struct CascadeSubset {
    /// Per-node computed styles in subtree walk order (parallel to the DOM
    /// walk from `subtree_root`, matching `CascadeResult::computed`'s layout).
    pub(crate) styles: Vec<raikiri_style::ComputedValues>,
}

/// Dynamic-content flags per running template — the four axes that make a
/// template's rendering depend on per-page state (design §7.3 lines
/// 1989-1993).
///
/// A template whose flags are all `false` is fully static and would qualify
/// for the post-M8 layout-result cache; today (M1〜M8) every template is
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
/// running-element. Since post-M8 static-template layout caching would key
/// on `(template_id, effective margin-box geometry)`, distinct running
/// elements (chapter-1's `<h1>` vs chapter-2's `<h1>`) naturally miss the
/// cache via distinct `template_id`s — the `content(text)` axis does not
/// add per-page dynamism the id-key doesn't already cover. The `#[non_exhaustive]`
/// catch-all in `detect_dynamic_flags`'s Content arm over-marks unknown
/// future keywords for safety (§7.3 line 2005 "correctness 優先").
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Populated by detect_dynamic_flags (walk over the template's \
              ContentComponent lists) and stored on ParsedRunningTemplate; \
              consumed by the post-M8 static-template layout-result cache."
)]
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
    /// would qualify for the post-M8 layout-result cache.
    #[allow(
        dead_code,
        reason = "Consumed by the post-M8 static-template layout-result cache \
                  (§7.3 lines 2014-2016)."
    )]
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
/// through raikiri-spike-96u.3; the variant populate landed with
/// raikiri-spike-96u.4 (canonical 6-variant shape per design doc §7.1
/// line 1913-1920). The field remains an empty `Vec` by default; the
/// register-site walker (later 96u-series task) will emit
/// `CounterIncrement` / `CounterReset` / `CounterSet` / `StringSet` /
/// `RegisterRunning` / `RegisterTarget` records under each subtree.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "Populated by the register-site walker (bd raikiri-spike-96u.4); \
              exercised via unit tests until then."
)]
pub(crate) struct ParsedRunningTemplate {
    /// The subtree root inside the document arena. `position: running(name)`
    /// removes this element from body flow; the pool holds the pointer.
    /// Doubles as the [`RunningTemplateId`] key
    /// ([`RunningTemplateId::new(subtree_root)`](RunningTemplateId::new)).
    pub(crate) subtree_root: NodeId,
    /// Pre-cascaded style block for every node under `subtree_root` (walk
    /// order). See [`CascadeSubset`].
    pub(crate) computed_styles: Arc<CascadeSubset>,
    /// GCPM directives that live inside the template subtree
    /// (`counter-increment`, `counter-reset`, `counter-set`, `string-set`,
    /// nested `running()` seeds if the spec/impl allows). Populated by the
    /// register-site walker (later 96u-series task); empty by default (see
    /// type-level `directives` field note).
    pub(crate) directives: Vec<GcpmDirective>,
    /// Which dynamic axes this template exercises (see [`DynamicFlags`]).
    pub(crate) dynamic_flags: DynamicFlags,
}

/// Tier-1 running-template store — parsed templates keyed by per-element
/// [`RunningTemplateId`], plus a name→ordered-pool index for `element(name)`
/// resolves.
///
/// Design doc §7.3 lines 1976-1980. Layout results are NOT cached; per-page
/// re-layout is unconditional in the M1〜M8 window
/// ([`layout_running_template`]). A future post-M8 tier-2 layout-result cache
/// would slot in alongside — this file does not implement it.
///
/// **Keying rationale**: `parsed_templates` is keyed by [`RunningTemplateId`]
/// (per-element id, wraps the subtree root's [`NodeId`]). The sibling
/// `name_pool` index maps each running-template name to the ids of every
/// element registered under `position: running(name)`, in **registration
/// (document) order**. Per-page selection — applying the
/// `element(<name>, [first|start|last|first-except]?)` selector keyword per
/// CSS GCPM 3 §1.2.2 (<https://www.w3.org/TR/css-gcpm-3/#element-syntax>) —
/// is a PageStream concern (bd raikiri-spike-96u.4); this store hands the
/// full ordered pool over and lets the caller filter. See the module-level
/// "`RunningTemplateId` = subtree_root" note for the shape justification.
///
/// See the module-level "Divergence from canonical shape" note for the
/// `pub(crate)` scoping rationale.
#[derive(Debug, Default)]
#[allow(
    dead_code,
    reason = "Producer path (register at cascade time) + consumer path \
              (element(name) resolve + per-page layout) both land with bd \
              raikiri-spike-96u.4; exercised via unit tests until then."
)]
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
    /// the caller invokes `register` in document order (the register-site
    /// walker landing with bd raikiri-spike-96u.4 walks the arena in
    /// document order, so this holds automatically). If violated, per-page
    /// pool selection emits the wrong element.
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
    #[allow(dead_code, reason = "Producer path lands with bd raikiri-spike-96u.4.")]
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
    #[allow(
        dead_code,
        reason = "Consumer path lands with bd raikiri-spike-96u.4 (M6 \
                  directive-apply pass); exercised via unit tests until then."
    )]
    pub(crate) fn resolve_element_pool(&self, name: &Symbol) -> &[RunningTemplateId] {
        self.name_pool.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Fetch the parsed template for an id (typically an id drawn from
    /// [`Self::resolve_element_pool`]).
    #[allow(dead_code, reason = "Consumer path lands with bd raikiri-spike-96u.4.")]
    pub(crate) fn get(&self, id: RunningTemplateId) -> Option<&ParsedRunningTemplate> {
        self.parsed_templates.get(&id)
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
/// the seed and OR-composing. No unit test exercises that multi-call fold
/// today — `detect_dynamic_flags_composes_multiple_axes` only composes
/// several axes within a *single* call's content list — a multi-call
/// OR-fold test is expected to land alongside the register-site walker
/// producer.
///
/// **`#[non_exhaustive]` handling**: [`ContentComponent`] is
/// `#[non_exhaustive]`; the future `ContentComponent::Element { name }`
/// variant (tracked as bd raikiri-spike-6z0) is NOT a dynamic-content signal
/// on its own — `element()` retrieves a static/dynamic template whose
/// dynamism is already captured in that template's own flags. The catch-all
/// arm therefore adds no flag. A downstream variant that would legitimately
/// flip a flag should be handled explicitly here — reviewer:spec should
/// challenge silent catch-all coverage of new variants.
///
/// **`Image` / `Contents` / `Quote` / `Leader`** (raikiri-spike-1us,
/// CSS Content 3 §2.2 / §2.3 / §2.4.2 / §2.5.1): each now has an explicit
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
/// but that's a layout-geometry input, not a content-dynamism axis — the
/// post-M8 cache key is `(template_id, effective margin-box geometry)`
/// (see the [`DynamicFlags`] type-level note), so geometry variance is
/// already covered by the cache key and doesn't need a flag here.
///
/// **reviewer:spec sign-off pending** on this "no dynamic flag" call (bd
/// raikiri-spike-5hp8) — the semantic read above is the implementer's, not
/// yet a spec-lens-confirmed classification.
#[allow(
    dead_code,
    reason = "Producer path is the register-site walker (bd \
              raikiri-spike-96u.4); exercised via unit tests until then."
)]
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
                    // element; the post-M8 layout cache would key on
                    // template_id, and distinct running elements naturally
                    // miss the cache without any dynamic-flag help.
                }
                // ContentTextKeyword is #[non_exhaustive]; a future variant
                // may or may not carry per-page dynamism. Over-mark for
                // safety (§7.3 line 2005 "correctness 優先"); reviewer:spec
                // should challenge whether a new keyword deserves an
                // explicit arm.
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
                // Static — page-independent per raikiri-spike-1us's semantic
                // read (image url() / quote nesting depth / leader glyph
                // resolve without per-page runtime state; `Contents`' own
                // descendants will be separately-walked arena nodes the
                // register-site walker is contracted to OR-fold on their
                // own account once that producer lands — see the type-level
                // doc note above for why this does NOT parallel the earlier
                // `Content { keyword: Before | After }` arm). reviewer:spec
                // sign-off pending on this classification (bd
                // raikiri-spike-5hp8).
            }
            // Non-exhaustive catch-all: reviewer:spec must challenge any new
            // ContentComponent variant that shouldn't fall through here.
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
              driver landing post-96u.4."
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
/// integration (post-96u.4).
#[derive(Debug, Clone, PartialEq)]
#[allow(
    dead_code,
    reason = "Return shape of layout_running_template; consumer landing with \
              the paint integration (post-96u.4)."
)]
pub(crate) struct MarginBoxLayoutResult {
    /// The template that was laid out.
    pub(crate) template_id: RunningTemplateId,
    /// The geometry it was laid out against (for cache-key parity if a
    /// future post-M8 tier-2 static-template cache lands).
    pub(crate) geometry: MarginBoxGeometry,
    /// Whether the source template was `dynamic_flags.is_fully_static()`.
    /// Post-M8 layout-result cache decision key — captured here so the
    /// eventual cache doesn't need to re-open the [`ParsedRunningTemplate`].
    pub(crate) source_was_static: bool,
}

/// Per-page re-layout entry point (design §7.3 lines 2007-2012).
///
/// **Always per-page** — the M1〜M8 flow is "no layout-result cache, always
/// re-layout" (§7.3 line 1979, 2000, 2014-2016). Fully-static templates
/// (`parsed.dynamic_flags.is_fully_static()`) are annotated on the return so
/// a post-M8 caller can decide whether to memoize; this function itself does
/// no caching.
///
/// **Stub for this task** — the return type is
/// [`MarginBoxLayoutResult`] rather than a paint fragment tree so the shape
/// stays dom-internal (see [`MarginBoxLayoutResult`] doc). Wiring to the
/// actual taffy/parley layout of the template subtree lands with the paint
/// integration.
#[allow(
    dead_code,
    reason = "Called from the per-page driver landing with bd \
              raikiri-spike-96u.4 / paint integration."
)]
pub(crate) fn layout_running_template(
    template_id: RunningTemplateId,
    parsed: &ParsedRunningTemplate,
    geometry: MarginBoxGeometry,
) -> MarginBoxLayoutResult {
    // Per-page re-layout is unconditional in M1〜M8; when the tier-2 cache
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
    use smol_str::SmolStr;

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
        // enum shape landed with raikiri-spike-96u.4).
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
        // template mis-qualify for the post-M8 static cache.
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
        // Explicit regression pin: the register-site walker (bd
        // raikiri-spike-96u.4) is expected to call register in document
        // order, and the pool must faithfully preserve that order — CSS
        // GCPM 3 §1.2.2 selector-keyword semantics (first/start/last/
        // first-except) all filter over an ordered pool. If a future
        // implementation switches to a HashSet / BTreeSet, this test breaks.
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
        // per running element; a post-M8 layout cache keyed on template_id
        // naturally misses across distinct elements. content(first-letter)
        // is derived from text and follows the same static classification.
        // Regression pin — pre-Codex the arm blanket-flipped for any Content
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
        // raikiri-spike-1us's 4 new variants (CSS Content 3 §2.2 / §2.3 /
        // §2.4.2 / §2.5.1) each get an explicit no-op arm now — pin that none
        // of them flip a dynamic flag. reviewer:spec sign-off pending on this
        // "no dynamic flag" classification (bd raikiri-spike-5hp8); this test
        // pins current behavior, not a spec-confirmed final answer.
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

    // ── layout_running_template (per-page re-layout stub) ──────────

    #[test]
    fn layout_running_template_returns_geometry_and_id_verbatim() {
        // Shape pin: the return threads through the input id/geometry unchanged.
        // Real per-page layout wiring lands with the paint integration (post-
        // 96u.4); the test verifies the invocation shape the design flow
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
        // Static template → source_was_static = true. Post-M8 layout-result
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
        // dynamic template mis-qualify for the post-M8 static cache).
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
