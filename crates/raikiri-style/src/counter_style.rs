//! `@counter-style` at-rule parsing + a name→rule registry + the CSS
//! "generate a counter" resolution algorithm.
//!
//! # Primary source
//!
//! CSS Counter Styles Level 3, §3 "The @counter-style Rule"
//! <https://www.w3.org/TR/css-counter-styles-3/#the-counter-style-rule> and
//! §2 "Counter Styles" (the `generate a counter` algorithm)
//! <https://www.w3.org/TR/css-counter-styles-3/#counter-styles>. Every
//! spec-derived branch below cites its own anchor; this header only states
//! the two top-level sections the rest of the module hangs off of.
//!
//! # Scope
//!
//! This module implements `@counter-style` parsing, the registry, and the
//! `generate a counter` algorithm. Wiring a formatted custom-counter string
//! into `raikiri-traits::TargetRegistry`'s resolution flow is deferred;
//! this module has **no dependency on raikiri-traits
//! and is not wired into that crate**. See [`resolve_custom_counter`]'s doc
//! for exactly what a caller gets back and why.
//!
//! What's implemented:
//!
//! - Full descriptor grammar: `system`,
//!   `negative`, `prefix`, `suffix`, `range`, `pad`, `fallback`, `symbols`,
//!   `additive-symbols`. `speak-as` is deliberately **not** in that list
//!   (audio-rendering concern, no bearing on the text-formatting use case
//!   this module serves) and is silently dropped like any other unsupported
//!   descriptor, per this crate's general convention (e.g. `@page`'s
//!   `size`/`marks`, `crate::ruletree`'s `@media`/`@supports`).
//! - The `generate a counter` algorithm (§2, quoted in full on
//!   [`generate_counter`]) for six of `system`'s seven values: `cyclic`,
//!   `numeric`, `alphabetic`, `symbolic`, `additive`, `fixed`. `extends` is
//!   parsed and stored (so a rule using it still round-trips through the
//!   registry) but composition against the extended style is **not**
//!   implemented — see [`CounterStyleSystem::Extends`]'s doc for the
//!   citation and rationale, following the general guidance to
//!   "start with whatever subset proves the parser + registry +
//!   generate-a-counter algorithm … widen from there."
//! - `<symbol> = <string> | <image> | <custom-ident>`
//!   (<https://www.w3.org/TR/css-counter-styles-3/#typedef-symbol>) is
//!   narrowed to the `<string>` / `<custom-ident>` alternatives — both
//!   "rendered as strings containing the same characters" per that anchor,
//!   which is exactly what a text-formatting consumer needs. `<image>`
//!   (`url(...)` / a `<gradient>`) has no textual rendering at all (spec:
//!   "rendered as inline replaced elements") and would need raikiri-paint
//!   integration this crate doesn't have; a symbol token that isn't a
//!   `<string>` or `<custom-ident>` simply fails [`parse_symbol`], which
//!   (like every other parse failure in this crate) silently drops the
//!   surrounding declaration.
//!
//! What's out of scope, beyond the bullet above:
//!
//! - `speak-as` (see above).
//!
//! # RuleTree wiring (origin-aware)
//!
//! [`crate::ruletree::RuleTree`] now owns a `counter_styles`
//! [`CounterStyleRegistry`], populated by every call to
//! [`crate::ruletree::RuleTree::add_stylesheet`] regardless of `origin` —
//! which covers both [`crate::ruletree::build_rule_tree`] (DOM `<style>`
//! element walk, always Author) and `raikiri`'s umbrella `build_cascaded`
//! (the actual production entry point, which calls `add_stylesheet` directly
//! with a mix of origins rather than going through `build_rule_tree`).
//! `add_stylesheet` runs [`parse_counter_style_rules`] as its own independent
//! second pass over the same `source` string rather than folding
//! `@counter-style` recognition into the existing `style_rules`/`page_rules`
//! parser — the single-pass-per-concern split this module started with (see
//! the "What's implemented" section above) is preserved; only the *insertion*
//! changed, not the parse strategy. Read the populated registry back via
//! [`crate::ruletree::RuleTree::counter_styles`].
//!
//! Each parsed rule is fed to [`CounterStyleRegistry::insert_with_origin`]
//! (`pub(crate)`, not [`CounterStyleRegistry::insert`] — see that method's
//! doc for why the plain `insert` stays origin-blind) together with
//! `add_stylesheet`'s own `origin` argument. The registry itself now tracks,
//! per name, which origin its current entry came from (type doc below), so
//! same-name resolution follows CSS Counter Styles L3 §3's "only one wins,
//! according to standard cascade rules" verbatim (origin-first, so Author
//! always beats UserAgent regardless of call order) without requiring the
//! caller to pre-filter by origin: a standalone `Origin::UserAgent`
//! `@counter-style` with no same-name `Origin::Author` rule is now available
//! (previously dropped unconditionally by an Author-only gate at the
//! `add_stylesheet` call site), while a same-name
//! `Origin::Author` rule still wins over any `Origin::UserAgent` rule
//! irrespective of which was inserted first.
//!
//! This closes the "no production consumer" gap this module previously had
//! for [`parse_counter_style_rules`] / [`CounterStyleRegistry`] (a registry
//! now exists alongside every `RuleTree`, populated from both origins), but
//! it is still one layer short of `counter()`/`counters()` actually
//! resolving during layout/paint: routing a matched [`CounterStyleRegistry`]
//! entry through [`resolve_custom_counter`] into
//! `raikiri-traits::TargetRegistry`'s resolution flow remains out of
//! scope, unchanged by this module (see the "Scope"
//! section above) — [`resolve_custom_counter`] itself is still exercised by
//! this module's own tests only.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use smol_str::SmolStr;

use crate::cascade::cascade_rank;
use crate::property::{is_reserved_custom_ident, parse_custom_ident};
use crate::ruletree::Origin;

// ---------------------------------------------------------------------------
// Symbol — `<symbol>` production, narrowed to `<string> | <custom-ident>`.
// ---------------------------------------------------------------------------

/// `<symbol>` (CSS Counter Styles L3
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-symbol>), narrowed to
/// the `<string>` and `<custom-ident>` alternatives — see the module doc's
/// "What's implemented" section for why `<image>` is excluded. Both retained
/// alternatives render identically ("a string containing the same characters
/// as" the literal / the identifier's spelling per the same anchor), so this
/// crate never needs to distinguish which alternative the author wrote —
/// hence a single newtype rather than a 2-variant enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterSymbol(pub SmolStr);

impl CounterSymbol {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// `<string>` (a plain quoted CSS string) or a bare `<custom-ident>`.
/// `<image>` (`url(...)`, a `<gradient>`, …) matches neither alternative and
/// this returns `None`, causing the surrounding declaration to be dropped by
/// its caller (see module doc).
fn parse_symbol(input: &mut Parser<'_, '_>) -> Option<CounterSymbol> {
    if let Ok(s) = input.try_parse(|i| i.expect_string().map(|s| s.as_ref().to_string())) {
        return Some(CounterSymbol(SmolStr::new(s)));
    }
    parse_custom_ident(input).map(CounterSymbol)
}

// ---------------------------------------------------------------------------
// system
// ---------------------------------------------------------------------------

/// `system` descriptor value (CSS Counter Styles L3 §3.1 "Counter System"
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-system>, value
/// grammar `cyclic | numeric | alphabetic | symbolic | additive | fixed
/// [<integer>]? | extends <counter-style-name>`, initial `symbolic`).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CounterStyleSystem {
    /// §3.1.1 <https://www.w3.org/TR/css-counter-styles-3/#cyclic-system> —
    /// cycles repeatedly through `symbols`. Does not use a negative sign.
    Cyclic,
    /// §3.1.5 <https://www.w3.org/TR/css-counter-styles-3/#numeric-system> —
    /// positional (place-value) numbering using `symbols` as digits.
    Numeric,
    /// §3.1.4
    /// <https://www.w3.org/TR/css-counter-styles-3/#alphabetic-system> —
    /// bijective (zero-less) numbering using `symbols` as digits.
    Alphabetic,
    /// §3.1.3 <https://www.w3.org/TR/css-counter-styles-3/#symbolic-system> —
    /// repeats/doubles through `symbols` on successive passes.
    Symbolic,
    /// §3.1.6 <https://www.w3.org/TR/css-counter-styles-3/#additive-system> —
    /// sign-value numbering (Roman-numeral-like) using `additive-symbols`.
    Additive,
    /// §3.1.2 <https://www.w3.org/TR/css-counter-styles-3/#fixed-system> —
    /// each value in a fixed window gets exactly one `symbols` entry;
    /// `first_symbol_value` is the optional leading `<integer>` (default 1,
    /// applied at parse time in [`parse_system`]).
    Fixed { first_symbol_value: i32 },
    /// §3.1.7 <https://www.w3.org/TR/css-counter-styles-3/#extends-system> —
    /// inherits the extended style's algorithm and any unspecified
    /// descriptors.
    ///
    /// **Composition is not implemented** (scope-cut, following the general
    /// guidance to "start with whatever subset … widen from there"). A rule using
    /// `extends` parses successfully (round-trips through
    /// [`CounterStyleRegistry`] with the extended name stored here) but
    /// [`resolve_custom_counter`] always falls through to the rule's
    /// `fallback` for it — see that function's doc for exactly which step of
    /// the algorithm this is modeled as. Composing the extended style's
    /// descriptors (including transitively, with the spec's own cycle
    /// handling: "If the specified counter style name isn't the name of any
    /// defined counter style, it must be treated as if it was extending the
    /// decimal counter style"
    /// <https://www.w3.org/TR/css-counter-styles-3/#extends-system>) is
    /// deferred to a follow-up.
    Extends(SmolStr),
}

/// `system: cyclic | numeric | alphabetic | symbolic | additive | fixed
/// [<integer>]? | extends <counter-style-name>`.
fn parse_system(input: &mut Parser<'_, '_>) -> Option<CounterStyleSystem> {
    let ident = input.expect_ident().ok()?.clone();
    Some(match ident.as_ref().to_ascii_lowercase().as_str() {
        "cyclic" => CounterStyleSystem::Cyclic,
        "numeric" => CounterStyleSystem::Numeric,
        "alphabetic" => CounterStyleSystem::Alphabetic,
        "symbolic" => CounterStyleSystem::Symbolic,
        "additive" => CounterStyleSystem::Additive,
        "fixed" => {
            // Optional leading `<integer>`, default 1 — §3.1.2.
            let first_symbol_value = input.try_parse(|i| i.expect_integer()).unwrap_or(1);
            CounterStyleSystem::Fixed { first_symbol_value }
        }
        "extends" => {
            let name = parse_counter_style_name_ref(input)?;
            CounterStyleSystem::Extends(name)
        }
        _ => return None,
    })
}

/// Whether `system` uses a negative sign when formatting a negative counter
/// value — CSS Counter Styles L3 §3.2 "Counter Symbols and Rendering:
/// negative"
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-negative>
/// verbatim: "a counter style uses a negative sign if its system value is
/// symbolic, alphabetic, numeric, additive, or extends if the extended
/// counter style itself uses a negative sign."
///
/// The `extends` clause is moot here: [`generate_counter`] never reaches
/// this function's result for an `Extends` system in the first place (its
/// step 3 dispatch returns `None` unconditionally, per the scope-cut on
/// [`CounterStyleSystem::Extends`]) — `false` is a safe placeholder that is
/// never observed.
fn system_uses_negative_sign(system: &CounterStyleSystem) -> bool {
    matches!(
        system,
        CounterStyleSystem::Numeric
            | CounterStyleSystem::Alphabetic
            | CounterStyleSystem::Symbolic
            | CounterStyleSystem::Additive
    )
}

// ---------------------------------------------------------------------------
// negative / prefix / suffix
// ---------------------------------------------------------------------------

/// `negative` descriptor value (CSS Counter Styles L3 §3.2
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-negative>,
/// value grammar `<symbol> <symbol>?`, initial `"-"` / no suffix). "The first
/// `<symbol>` … is prepended to the representation when the counter value is
/// negative. The second `<symbol>`, if specified, is appended."
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NegativeDescriptor {
    pub prefix: CounterSymbol,
    pub suffix: Option<CounterSymbol>,
}

impl Default for NegativeDescriptor {
    fn default() -> Self {
        Self {
            prefix: CounterSymbol(SmolStr::new("-")),
            suffix: None,
        }
    }
}

fn parse_negative(input: &mut Parser<'_, '_>) -> Option<NegativeDescriptor> {
    let prefix = parse_symbol(input)?;
    let suffix = input.try_parse(|i| parse_symbol(i).ok_or(())).ok();
    Some(NegativeDescriptor { prefix, suffix })
}

// ---------------------------------------------------------------------------
// pad
// ---------------------------------------------------------------------------

/// `pad` descriptor value (CSS Counter Styles L3 §3.6
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-pad>, value
/// grammar `<integer [0,∞]> && <symbol>` — `&&` means both components are
/// required, in **either** order — initial `0 ""`).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadDescriptor {
    pub min_length: u32,
    pub symbol: CounterSymbol,
}

impl Default for PadDescriptor {
    fn default() -> Self {
        Self {
            min_length: 0,
            symbol: CounterSymbol(SmolStr::new("")),
        }
    }
}

/// `<integer [0,∞]> && <symbol>` — shared CSS grammar shape between `pad`
/// (§3.6) and each tuple of `additive-symbols` (§3.8): `&&` means both
/// components are required, in **either** order. Factored out of
/// [`parse_pad`] and [`parse_weight_symbol_pair`] so the two `&&`-ordering
/// branches exist exactly once (quality finding: those two functions used
/// to duplicate this shape independently).
fn parse_nonneg_int_and_symbol(input: &mut Parser<'_, '_>) -> Option<(i32, CounterSymbol)> {
    if let Ok(n) = input.try_parse(|i| i.expect_integer()) {
        if n < 0 {
            return None;
        }
        let symbol = parse_symbol(input)?;
        return Some((n, symbol));
    }
    let symbol = parse_symbol(input)?;
    let n = input.expect_integer().ok()?;
    if n < 0 {
        return None;
    }
    Some((n, symbol))
}

fn parse_pad(input: &mut Parser<'_, '_>) -> Option<PadDescriptor> {
    let (n, symbol) = parse_nonneg_int_and_symbol(input)?;
    Some(PadDescriptor {
        min_length: n as u32,
        symbol,
    })
}

/// Prepend copies of `pad.symbol` until `repr` reaches `pad.min_length`,
/// reserving room for a negative sign that will be wrapped around `repr`
/// afterward (step 5, back in [`generate_counter`]) if `negative_reserved`
/// is nonzero.
///
/// CSS Counter Styles L3 §3.6 verbatim (full paragraph, all 3 sentences):
/// "Let difference be the provided integer minus the number of grapheme
/// clusters in the initial representation for the counter value. (Note
/// that, per the algorithm to generate a counter representation, this
/// occurs before adding prefixes/suffixes/negatives.) If the counter value
/// is negative and the counter style uses a negative sign, further reduce
/// difference by the number of grapheme clusters in the counter style's
/// negative descriptor's symbol(s). If difference is greater than zero,
/// prepend difference copies of the specified symbol to the
/// representation." `negative_reserved` is the caller-computed "grapheme
/// clusters in the negative descriptor's symbol(s)" term from the middle
/// sentence — `0` when the counter value isn't negative or the style
/// doesn't use a negative sign (the sentence's own gating condition), so
/// this function doesn't need to re-derive that condition itself.
///
/// **Scope-cut**: both `repr`'s length and `negative_reserved` count
/// `.chars()` (Unicode scalar values), not grapheme clusters (CSS Text 3
/// <https://www.w3.org/TR/css-text-3/#grapheme-cluster>). A combining
/// character sequence or ZWJ emoji sequence in a symbol would count as more
/// than one "cluster" here, over-padding slightly. `unicode-segmentation`
/// (true grapheme clustering) is not a workspace dependency; the ASCII/BMP
/// case this crate's tests exercise is unaffected, and true clustering is
/// deferred per "start with subset, widen from there"
/// rather than adding a dependency for it now.
fn apply_pad(pad: &PadDescriptor, repr: String, negative_reserved: i64) -> String {
    let len = repr.chars().count() as i64;
    let diff = i64::from(pad.min_length) - len - negative_reserved;
    if diff > 0 {
        let mut out = pad.symbol.as_str().repeat(diff as usize);
        out.push_str(&repr);
        out
    } else {
        repr
    }
}

/// Grapheme-cluster (approximated as `.chars()`, see [`apply_pad`]'s
/// "Scope-cut" note) count of `negative`'s symbol(s) — the prefix, plus the
/// optional suffix when present. This is exactly the "grapheme clusters in
/// the counter style's negative descriptor's symbol(s)" term CSS Counter
/// Styles L3 §3.6 (quoted in full on [`apply_pad`]) subtracts from the pad
/// `difference` when the counter value is negative and the style uses a
/// negative sign.
fn negative_descriptor_len(negative: &NegativeDescriptor) -> i64 {
    let mut len = negative.prefix.as_str().chars().count() as i64;
    if let Some(suffix) = &negative.suffix {
        len += suffix.as_str().chars().count() as i64;
    }
    len
}

// ---------------------------------------------------------------------------
// fallback / <counter-style-name> references
// ---------------------------------------------------------------------------

/// `<counter-style-name>` (CSS Counter Styles L3 §3 dfn
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name>)
/// verbatim: "`<counter-style-name>` is a `<custom-ident>` that is not an
/// ASCII case-insensitive match for `none`." This is the *reference* form —
/// used by `fallback` and `system: extends`. It intentionally does **not**
/// exclude `decimal`/`disc`/`circle`/`square`/`disclosure-open`/
/// `disclosure-closed`: the same anchor's next sentence says those "are
/// valid `<counter-style-name>`s, but are invalid when used here to name a
/// counter style rule" — i.e. the extra exclusion applies only to the rule's
/// own name, parsed separately by [`parse_counter_style_rule_name`].
fn parse_counter_style_name_ref(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if ident.eq_ignore_ascii_case("none") || is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `fallback` descriptor value (CSS Counter Styles L3 §3.7
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-fallback>,
/// value `<counter-style-name>`, initial `decimal`).
fn parse_fallback(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    parse_counter_style_name_ref(input)
}

// ---------------------------------------------------------------------------
// range
// ---------------------------------------------------------------------------

/// One bound of a [`RangeEntry`] — `<integer>` or the `infinite` keyword.
/// Which infinity `Infinite` denotes (−∞ vs +∞) depends on its position
/// (`lower` vs `upper`) in the containing [`RangeEntry`], per CSS Counter
/// Styles L3 §3.5
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-range>: "If
/// `infinite` is used as the first value, it represents negative infinity;
/// if used as the second value, it represents positive infinity."
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeLimit {
    Infinite,
    Finite(i32),
}

/// One `[<integer> | infinite]{2}` pair from the `range` descriptor's
/// comma-separated list — CSS Counter Styles L3 §3.5 (anchor above): "the
/// first value is the lower bound and the second value is the upper bound.
/// This range is inclusive — it contains both bounds."
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeEntry {
    pub lower: RangeLimit,
    pub upper: RangeLimit,
}

impl RangeEntry {
    fn contains(&self, value: i64) -> bool {
        let lower_ok = match self.lower {
            RangeLimit::Infinite => true,
            RangeLimit::Finite(l) => value >= i64::from(l),
        };
        let upper_ok = match self.upper {
            RangeLimit::Infinite => true,
            RangeLimit::Finite(u) => value <= i64::from(u),
        };
        lower_ok && upper_ok
    }
}

/// `range` descriptor value (CSS Counter Styles L3 §3.5, anchor above; value
/// grammar `[[<integer> | infinite]{2}]# | auto`, initial `auto`).
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CounterRange {
    #[default]
    Auto,
    /// Non-empty per the `#` (one-or-more, comma-separated) multiplier —
    /// [`parse_range`] never produces an empty `List`.
    List(Vec<RangeEntry>),
}

fn parse_range_limit<'i>(input: &mut Parser<'i, '_>) -> Result<RangeLimit, ParseError<'i, ()>> {
    if input
        .try_parse(|i| i.expect_ident_matching("infinite"))
        .is_ok()
    {
        Ok(RangeLimit::Infinite)
    } else {
        Ok(RangeLimit::Finite(input.expect_integer()?))
    }
}

fn parse_range(input: &mut Parser<'_, '_>) -> Option<CounterRange> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(CounterRange::Auto);
    }
    let entries: Vec<RangeEntry> = input
        .parse_comma_separated(|i| {
            let lower = parse_range_limit(i)?;
            let upper = parse_range_limit(i)?;
            Ok(RangeEntry { lower, upper })
        })
        .ok()?;
    // cov:ignore: structurally unreachable, not merely untested. cssparser
    // 0.37.0's `parse_comma_separated` (the non-`_ignoring_errors` variant
    // used above) either returns `Err` — propagated by the `.ok()?` right
    // above, already exercised by this crate's malformed-range tests — or
    // an `Ok` `Vec` that its own source comment guarantees is non-empty:
    // "we always push at least one item if parsing succeeds" (`cssparser-
    // 0.37.0/src/parser.rs`, `parse_comma_separated_internal`: on the first
    // segment's `Err` it returns `Err` immediately rather than continuing,
    // so `Ok` is only ever reached after at least one successful push).
    // There is no CSS input that reaches this branch with `Ok(vec![])`.
    if entries.is_empty() {
        return None;
    }
    // §3.5 verbatim: "If the lower bound of any range is higher than the
    // upper bound, the entire descriptor is invalid and must be ignored" —
    // i.e. the whole `range` descriptor (not just one entry) reverts to its
    // initial value `auto`. Modeled here by returning `None`, which
    // `CounterStyleDeclParser::parse_value` (below) treats as "drop this
    // declaration", leaving `CounterStyleRule::range` at its
    // `#[derive(Default)]` value `Auto`.
    for entry in &entries {
        if let (RangeLimit::Finite(l), RangeLimit::Finite(u)) = (entry.lower, entry.upper)
            && l > u
        {
            return None;
        }
    }
    Some(CounterRange::List(entries))
}

/// The `range` a rule is actually defined over — either its own explicit
/// `range` descriptor, or (for `auto`) the per-system default range. CSS
/// Counter Styles L3 §3.5 "range"
/// (<https://www.w3.org/TR/css-counter-styles-3/#counter-style-range>)
/// verbatim: "For cyclic, numeric, and fixed systems, the range is negative
/// infinity to positive infinity."
///
/// **`fixed` is grouped with the unbounded systems here, not given a finite
/// window** — an earlier version of this function computed a finite
/// `first_symbol_value .. first_symbol_value + symbols.len() - 1` window for
/// `fixed` instead, which (a) misattributed §3.1.2's symbol-exhaustion
/// behavior to this §3.5 range check when the spec's own §3.5 text quoted
/// above puts `fixed` in the *unbounded* group, and (b) could overflow: near
/// `i32::MAX`, `first_symbol_value.saturating_add(symbols.len()).saturating_sub(1)`
/// could invert into `lower > upper`, making [`RangeEntry::contains`] return
/// `false` for every value — silently reporting "always out of range" for a
/// rule `fixed_repr` could otherwise represent. `fixed`'s actual
/// window-exhaustion behavior — "further values cannot be represented by
/// this counter style, and must instead be represented by the fallback
/// counter style" — is already, independently enforced by [`fixed_repr`]'s
/// own bounds check (quoted in full there); it just hands off to the same
/// `fallback` at step 3 of [`generate_counter`] instead of at this
/// function's step 2, with an identical end result for every value this
/// crate has tests for.
///
/// - cyclic / numeric / fixed: `Infinite..Infinite`, per the quoted sentence
///   above.
/// - alphabetic (§3.1.4) / symbolic (§3.1.3): "1 to positive infinity" (both
///   are documented above as strictly-positive-only).
/// - additive (§3.1.6): "0 to positive infinity".
/// - extends: moot here — see [`CounterStyleSystem::Extends`]'s doc for why
///   [`generate_counter`] never depends on this function's answer for it.
///   `Infinite..Infinite` is returned so that, if it *were* consulted, step 2
///   of the algorithm (the range check) would never itself trigger the
///   fallback for an `extends` rule — the scope-cut fallback is always
///   attributed to step 3 instead, keeping "which step handled it" legible
///   in [`generate_counter`].
fn auto_range(system: &CounterStyleSystem) -> RangeEntry {
    match system {
        CounterStyleSystem::Cyclic
        | CounterStyleSystem::Numeric
        | CounterStyleSystem::Fixed { .. }
        | CounterStyleSystem::Extends(_) => RangeEntry {
            lower: RangeLimit::Infinite,
            upper: RangeLimit::Infinite,
        },
        CounterStyleSystem::Alphabetic | CounterStyleSystem::Symbolic => RangeEntry {
            lower: RangeLimit::Finite(1),
            upper: RangeLimit::Infinite,
        },
        CounterStyleSystem::Additive => RangeEntry {
            lower: RangeLimit::Finite(0),
            upper: RangeLimit::Infinite,
        },
    }
}

fn rule_contains_value(rule: &CounterStyleRule, value: i32) -> bool {
    let v = i64::from(value);
    match &rule.range {
        CounterRange::List(entries) => entries.iter().any(|e| e.contains(v)),
        CounterRange::Auto => auto_range(&rule.system).contains(v),
    }
}

// ---------------------------------------------------------------------------
// symbols / additive-symbols
// ---------------------------------------------------------------------------

/// `symbols` descriptor value (CSS Counter Styles L3 §3.8
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-symbols>,
/// value `<symbol>+`).
fn parse_symbols(input: &mut Parser<'_, '_>) -> Option<Vec<CounterSymbol>> {
    let mut out = Vec::new();
    while let Ok(sym) = input.try_parse(|i| parse_symbol(i).ok_or(())) {
        out.push(sym);
    }
    if out.is_empty() { None } else { Some(out) }
}

fn parse_weight_symbol_pair<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(i32, CounterSymbol), ParseError<'i, ()>> {
    parse_nonneg_int_and_symbol(input).ok_or_else(|| input.new_custom_error(()))
}

/// `additive-symbols` descriptor value (CSS Counter Styles L3 §3.8, anchor
/// above; value `[<integer [0,∞]> && <symbol>]#`).
///
/// §3.8 verbatim on ordering: "Each weight must be a non-negative integer,
/// and the additive tuples must be specified in order of strictly descending
/// weight; otherwise, the declaration is invalid and must be ignored." —
/// modeled the same way as [`parse_range`]'s analogous rule: `None` here
/// leaves [`CounterStyleRule::additive_symbols`] at its default (empty)
/// value via [`CounterStyleDeclParser`]'s normal invalid-declaration
/// handling.
fn parse_additive_symbols(input: &mut Parser<'_, '_>) -> Option<Vec<(i32, CounterSymbol)>> {
    let tuples: Vec<(i32, CounterSymbol)> =
        input.parse_comma_separated(parse_weight_symbol_pair).ok()?;
    // cov:ignore: structurally unreachable — same cssparser
    // `parse_comma_separated` non-empty-on-`Ok` guarantee cited in
    // `parse_range`'s identical guard, which this one mirrors.
    if tuples.is_empty() {
        return None;
    }
    for pair in tuples.windows(2) {
        if pair[0].0 <= pair[1].0 {
            return None;
        }
    }
    Some(tuples)
}

// ---------------------------------------------------------------------------
// CounterStyleRule + the per-declaration parser that builds one
// ---------------------------------------------------------------------------

/// A parsed `@counter-style` rule — one entry per descriptor CSS Counter
/// Styles L3 §3 defines, minus `speak-as` (see module doc). Fields are
/// `pub` for direct read/construction (this is a fresh, self-contained
/// module with no accumulated write-path history to guard against, unlike
/// e.g. [`crate::rule::StyleRule`]); the one invariant this type cares about
/// — "only a spec-valid rule enters a registry" — is enforced at the
/// [`CounterStyleRegistry::insert`] boundary via [`CounterStyleRule::is_valid`],
/// not by field privacy.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterStyleRule {
    /// The rule's own `<counter-style-name>`, already checked against the
    /// naming-specific reserved list (see
    /// [`parse_counter_style_rule_name`]) — this is the registry key.
    pub name: SmolStr,
    pub system: CounterStyleSystem,
    pub negative: NegativeDescriptor,
    /// Stored for completeness (parsed per CSS Counter Styles L3 §3.3
    /// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-prefix>)
    /// but **never applied by [`resolve_custom_counter`]**. §3.3 verbatim:
    /// "Prefixes are only added by the algorithm for constructing the
    /// default contents of the `::marker` pseudo-element; the prefix is not
    /// added automatically when the `counter()` or `counters()` functions
    /// are used." [`resolve_custom_counter`] implements the `counter()`
    /// -context `generate a counter` algorithm (§2), which has no
    /// prefix/suffix step at all — this is that omission's citation, not an
    /// oversight. A future `::marker`-string consumer would read this field
    /// directly and prepend it itself.
    pub prefix: CounterSymbol,
    /// See [`Self::prefix`] — §3.4
    /// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-suffix>
    /// states the same "only `::marker`, not `counter()`/`counters()`"
    /// scoping for `suffix`, symmetrically unapplied here.
    pub suffix: CounterSymbol,
    pub range: CounterRange,
    pub pad: PadDescriptor,
    pub fallback: SmolStr,
    pub symbols: Vec<CounterSymbol>,
    pub additive_symbols: Vec<(i32, CounterSymbol)>,
}

impl CounterStyleRule {
    /// A fresh rule with every descriptor at its spec-defined initial value
    /// (CSS Counter Styles L3 §3's descriptor table — `system: symbolic`,
    /// `negative: "-"`, `prefix: ""`, `suffix: ". "`, `range: auto`,
    /// `pad: 0 ""`, `fallback: decimal`; `symbols` / `additive_symbols` have
    /// no initial value per the spec table ("n/a") and start empty here).
    pub fn new(name: SmolStr) -> Self {
        Self {
            name,
            system: CounterStyleSystem::Symbolic,
            negative: NegativeDescriptor::default(),
            prefix: CounterSymbol(SmolStr::new("")),
            suffix: CounterSymbol(SmolStr::new(". ")),
            range: CounterRange::Auto,
            pad: PadDescriptor::default(),
            fallback: SmolStr::new("decimal"),
            symbols: Vec::new(),
            additive_symbols: Vec::new(),
        }
    }

    /// Whole-rule validity — CSS Counter Styles L3 §3.8 verbatim: "If the
    /// counter style's system is such [requiring ≥1 or ≥2 symbols], and the
    /// symbols descriptor has only a single entry [or is absent], the
    /// `@counter-style` rule does not define a counter style"
    /// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-symbols>;
    /// plus the `additive` minimum ("at least one additive tuple", §3.1.6
    /// <https://www.w3.org/TR/css-counter-styles-3/#additive-system>) and
    /// the `extends` exclusivity ("An `extends` system … cannot appear along
    /// with a `symbols` or `additive-symbols` descriptor", §3.1.7
    /// <https://www.w3.org/TR/css-counter-styles-3/#extends-system>).
    ///
    /// [`CounterStyleRegistry::insert`] is the only call site — a rule that
    /// fails this check is silently dropped there, per this crate's general
    /// "spec-invalid → silently dropped" convention (`@page`'s invalid
    /// selector handling is the sibling precedent).
    pub fn is_valid(&self) -> bool {
        match &self.system {
            CounterStyleSystem::Cyclic
            | CounterStyleSystem::Symbolic
            | CounterStyleSystem::Fixed { .. } => !self.symbols.is_empty(),
            CounterStyleSystem::Numeric | CounterStyleSystem::Alphabetic => self.symbols.len() >= 2,
            CounterStyleSystem::Additive => !self.additive_symbols.is_empty(),
            CounterStyleSystem::Extends(_) => {
                self.symbols.is_empty() && self.additive_symbols.is_empty()
            }
        }
    }
}

/// One successfully-parsed descriptor, tagged by which one it is — the
/// per-declaration output of [`CounterStyleDeclParser`], folded into a
/// [`CounterStyleRule`] by [`build_rule`] (later declarations of the same
/// descriptor win, matching ordinary CSS declaration-list semantics — the
/// same "later wins" fold [`crate::rule::parse_declaration_block`]'s callers
/// rely on for the cascade).
enum ParsedDescriptor {
    System(CounterStyleSystem),
    Negative(NegativeDescriptor),
    Prefix(CounterSymbol),
    Suffix(CounterSymbol),
    Range(CounterRange),
    Pad(PadDescriptor),
    Fallback(SmolStr),
    Symbols(Vec<CounterSymbol>),
    AdditiveSymbols(Vec<(i32, CounterSymbol)>),
}

fn parse_descriptor_value(name: &str, input: &mut Parser<'_, '_>) -> Option<ParsedDescriptor> {
    match name.to_ascii_lowercase().as_str() {
        "system" => parse_system(input).map(ParsedDescriptor::System),
        "negative" => parse_negative(input).map(ParsedDescriptor::Negative),
        "prefix" => parse_symbol(input).map(ParsedDescriptor::Prefix),
        "suffix" => parse_symbol(input).map(ParsedDescriptor::Suffix),
        "range" => parse_range(input).map(ParsedDescriptor::Range),
        "pad" => parse_pad(input).map(ParsedDescriptor::Pad),
        "fallback" => parse_fallback(input).map(ParsedDescriptor::Fallback),
        "symbols" => parse_symbols(input).map(ParsedDescriptor::Symbols),
        "additive-symbols" => parse_additive_symbols(input).map(ParsedDescriptor::AdditiveSymbols),
        // `speak-as` and anything unrecognized: silently dropped (module doc
        // "What's implemented").
        _ => None,
    }
}

/// Per-declaration parser for the `@counter-style` block's `<declaration-list>`
/// — mirrors [`mod@crate::rule`]'s `DeclParser` shape exactly (same
/// `RuleBodyItemParser` wiring, same "unsupported name / invalid value →
/// `Err` → whole declaration silently dropped by `RuleBodyParser`'s error
/// recovery" behavior), specialized to counter-style descriptor names
/// instead of CSS property names.
struct CounterStyleDeclParser;

impl<'i> DeclarationParser<'i> for CounterStyleDeclParser {
    type Declaration = ParsedDescriptor;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<ParsedDescriptor, ParseError<'i, Self::Error>> {
        let descriptor = parse_descriptor_value(name.as_ref(), input)
            .ok_or_else(|| input.new_custom_error(()))?;
        // Exhaustive consumption, same rationale as `DeclParser::parse_value`:
        // trailing garbage after a valid value must reject the whole
        // declaration rather than silently accepting a prefix.
        input.expect_exhausted().map_err(
            |e: cssparser::BasicParseError<'i>| -> ParseError<'i, Self::Error> { e.into() },
        )?;
        Ok(descriptor)
    }
}

// Nested at-rules / qualified rules inside a `@counter-style` block are not
// part of the L3 grammar (`<declaration-list>` only) — no-op, matching
// `DeclParser`'s identical arms for the same reason.
impl<'i> AtRuleParser<'i> for CounterStyleDeclParser {
    type Prelude = ();
    type AtRule = ParsedDescriptor;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for CounterStyleDeclParser {
    type Prelude = ();
    type QualifiedRule = ParsedDescriptor;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, ParsedDescriptor, ()> for CounterStyleDeclParser {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

fn parse_counter_style_block(input: &mut Parser<'_, '_>) -> Vec<ParsedDescriptor> {
    let mut parser = CounterStyleDeclParser;
    RuleBodyParser::new(input, &mut parser).flatten().collect()
}

fn build_rule(name: SmolStr, descriptors: Vec<ParsedDescriptor>) -> CounterStyleRule {
    let mut rule = CounterStyleRule::new(name);
    for d in descriptors {
        match d {
            ParsedDescriptor::System(v) => rule.system = v,
            ParsedDescriptor::Negative(v) => rule.negative = v,
            ParsedDescriptor::Prefix(v) => rule.prefix = v,
            ParsedDescriptor::Suffix(v) => rule.suffix = v,
            ParsedDescriptor::Range(v) => rule.range = v,
            ParsedDescriptor::Pad(v) => rule.pad = v,
            ParsedDescriptor::Fallback(v) => rule.fallback = v,
            ParsedDescriptor::Symbols(v) => rule.symbols = v,
            ParsedDescriptor::AdditiveSymbols(v) => rule.additive_symbols = v,
        }
    }
    rule
}

// ---------------------------------------------------------------------------
// Stylesheet-level extraction — `@counter-style` rules only.
// ---------------------------------------------------------------------------

/// `<counter-style-name>` in **naming** position (the rule's own name, right
/// after `@counter-style`). Same base exclusion as
/// [`parse_counter_style_name_ref`] (custom-ident, not `none`), plus the
/// six-keyword exclusion CSS Counter Styles L3 §3 dfn states immediately
/// after the `<counter-style-name>` definition
/// (<https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name>):
/// "The keywords `decimal`, `disc`, `square`, `circle`, `disclosure-open`,
/// and `disclosure-closed` are valid `<counter-style-name>`s, but are
/// invalid when used here to name a counter style rule; doing so makes the
/// rule invalid."
fn parse_counter_style_rule_name<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SmolStr, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    let reserved_here = matches!(
        ident.as_ref().to_ascii_lowercase().as_str(),
        "decimal" | "disc" | "square" | "circle" | "disclosure-open" | "disclosure-closed"
    );
    if reserved_here || ident.eq_ignore_ascii_case("none") || is_reserved_custom_ident(&ident) {
        return Err(input.new_custom_error(()));
    }
    // Grammar is `@counter-style <counter-style-name> { … }` — nothing else
    // is permitted in the prelude.
    input.expect_exhausted()?;
    Ok(SmolStr::new(ident.as_ref()))
}

/// Top-level parsed item this module's stylesheet scan can produce. `Style`
/// (any qualified rule) is never actually constructed — see
/// [`CounterStyleSheetParser`]'s `QualifiedRuleParser` impl — but
/// `cssparser::StyleSheetParser` requires `AtRuleParser::AtRule` and
/// `QualifiedRuleParser::QualifiedRule` to be the same type, mirroring
/// `crate::ruletree`'s identical `ParsedRule` shape for the same reason.
enum TopLevelItem {
    CounterStyle(SmolStr, Vec<ParsedDescriptor>),
}

/// Stylesheet-level parser: accepts only `@counter-style` at-rules, drops
/// everything else (other at-rules, all qualified/style rules) — this
/// module has no interest in anything but `@counter-style`. See the module
/// doc's "What's out of scope" bullet on why this runs its own independent
/// scan rather than extending [`crate::ruletree`]'s `StyleRuleParser`.
struct CounterStyleSheetParser;

impl<'i> AtRuleParser<'i> for CounterStyleSheetParser {
    type Prelude = SmolStr;
    type AtRule = TopLevelItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("counter-style") {
            parse_counter_style_rule_name(input)
        } else {
            // @media / @page / @font-face / etc — not this module's
            // concern; cssparser's error recovery skips the whole block.
            Err(input.new_custom_error(()))
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        let descriptors = parse_counter_style_block(input);
        Ok(TopLevelItem::CounterStyle(prelude, descriptors))
    }
}

impl<'i> QualifiedRuleParser<'i> for CounterStyleSheetParser {
    type Prelude = ();
    type QualifiedRule = TopLevelItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        // Style rules aren't this module's concern — always reject so
        // cssparser's error recovery drops the whole rule.
        Err(input.new_custom_error(()))
    }

    // cov:ignore: structurally unreachable, not merely untested.
    // `parse_prelude` above always returns `Err`, so cssparser's
    // qualified-rule dispatch never calls this method — it exists only
    // because `QualifiedRuleParser` requires an implementation.
    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        Err(input.new_custom_error(()))
    }
}

/// Parse every `@counter-style` at-rule out of `source` (a full stylesheet
/// text — e.g. the concatenated text of a `<style>` element, same unit
/// [`crate::ruletree::RuleTree::add_stylesheet`] takes). Everything else in
/// `source` (style rules, other at-rules) is silently ignored. A rule that
/// fails [`CounterStyleRule::is_valid`] is **not** included — matches
/// `RuleTree`'s "invalid rule → dropped" convention for `@page`.
pub fn parse_counter_style_rules(source: &str) -> Vec<CounterStyleRule> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut sheet_parser = CounterStyleSheetParser;
    let mut out = Vec::new();
    for item in StyleSheetParser::new(&mut parser, &mut sheet_parser).flatten() {
        let TopLevelItem::CounterStyle(name, descriptors) = item;
        let rule = build_rule(name, descriptors);
        if rule.is_valid() {
            out.push(rule);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CounterStyleRegistry
// ---------------------------------------------------------------------------

/// Name → [`CounterStyleRule`] registry.
///
/// CSS Counter Styles L3 §3 (anchor above) verbatim on same-name rules: "If
/// multiple `@counter-style` rules are defined with the same name, only one
/// wins, according to standard cascade rules. … `@counter-style` rules
/// cascade 'atomically': if one replaces another of the same name, it
/// replaces it *entirely*, rather than just replacing the specific
/// descriptors it specifies."
///
/// Both halves of that quote are implemented here. "Atomically" is a plain
/// overwrite — whole `CounterStyleRule` values are stored, never merged
/// field-by-field. "Standard cascade rules" (origin first, then source
/// order within an origin) is implemented by recording, per name, which
/// [`Origin`] the currently-stored rule came from
/// (this field was origin-blind before — see
/// [`Self::insert`]'s doc for the one remaining origin-blind entry point)
/// and consulting that origin on every subsequent insert of the same name:
///
/// - [`Self::insert_with_origin`] (`pub(crate)`, used by
///   [`crate::ruletree::RuleTree::add_stylesheet`]) applies the actual
///   precedence via [`crate::cascade::cascade_rank`]: a new rule overwrites
///   unless it is itself outranked by the currently-stored one. Concretely,
///   for the origins every current in-repo call site of
///   [`crate::ruletree::RuleTree::add_stylesheet`] actually passes it
///   (`Origin::UserAgent` / `Origin::Author` / `Origin::User` —
///   `@counter-style` never comes from the
///   crate-private HTML presentational-hint path
///   (`push_img_dimension_hints`, `crates/raikiri-style/src/cascade.rs`),
///   so [`Origin::AuthorPresentationalHint`] does not reach here via any
///   caller in this crate today, even though [`Origin`] itself has 4
///   variants). [`Origin::User`] (added
///   alongside [`Origin::AuthorPresentationalHint`]) was likewise
///   unreachable here at the time it was added —
///   no caller anywhere routed to `Origin::User` yet. That later changed:
///   consumer-provided `extra_stylesheets`
///   is now tagged `Origin::User` end-to-end (via raikiri-html's retag +
///   umbrella's `stylesheet_kind_to_origin`), and
///   [`crate::ruletree::RuleTree::add_stylesheet`] runs
///   [`parse_counter_style_rules`] +
///   [`Self::insert_with_origin`] against the *same* `source`/`origin` it
///   was given for style rules — so an `@counter-style` rule inside a
///   consumer's `extra_stylesheets` string now reaches here tagged
///   `Origin::User` too, in production. Note that `add_stylesheet` is
///   `pub fn` with an unconstrained `origin: Origin` parameter, so this is
///   a fact about current callers, not a structural guarantee — an
///   external caller passing `Origin::AuthorPresentationalHint` directly
///   would reach `insert_with_origin` and be resolved correctly by the
///   rank-based logic below regardless — a new `Origin::Author` rule always overwrites,
///   regardless of what's currently stored (same rank as an existing
///   Author entry, or outranks an existing UserAgent one); a new
///   `Origin::UserAgent` rule overwrites only when nothing is stored yet or
///   the stored entry is itself `Origin::UserAgent` (so the last
///   `Origin::UserAgent` insert of a name wins among same-origin entries,
///   matching the source-order tie-break), and is dropped when the stored
///   entry is `Origin::Author` (Author always beats UserAgent, independent
///   of call order — CSS Cascading L4 §"cascade-origin"
///   (<https://www.w3.org/TR/css-cascade-4/#cascade-origin>)). Reuses
///   [`crate::cascade::cascade_rank`] rather than a local rank fn — same
///   sibling-arm convention [`crate::page::cascade_page`] already follows
///   for `@page` (that function's doc, "Sibling arm convention"):
///   `@counter-style` and style-rule origin ordering are
///   the same CSS Cascading L4 mechanism, so a second copy of the
///   `Origin -> u8` mapping would just be drift risk. The call always fixes
///   `important` to `false`: CSS Counter Styles L3 §3's "standard cascade
///   rules" has no `!important`-equivalent concept for `@counter-style` at
///   all, so there's nothing to pass through — but `false` isn't an
///   arbitrary placeholder either, it's specifically the *non-important*
///   half of [`crate::cascade::cascade_rank`]'s ranking (UA < User <
///   AuthorPresentationalHint < Author,
///   `cascade_rank` doc has the exact values), which is the half that
///   actually matches §3's origin order;
///   the other half ([`crate::cascade::cascade_rank`] with
///   `important: true`) inverts precedence and would be wrong here.
///   Resolution is by rank, not a hardcoded `Author`/`UserAgent` pair,
///   specifically so adding a variant to [`Origin`] (`#[non_exhaustive]`)
///   is a compile error at [`crate::cascade::cascade_rank`]'s own `match`
///   — not a silently-wrong precedence here. This has now happened twice:
///   once when [`Origin::AuthorPresentationalHint`] was added, and once
///   when [`Origin::User`] was added (which later gained a production
///   producer, [`Origin::User`]'s
///   doc has the status) — both times `cascade_rank`'s `match` had to be
///   updated to stay exhaustive, but this function's logic needed no
///   change (rank-based, not per-variant — see above).
/// - [`Self::insert`] (the `pub` entry point, unchanged since before
///   origin-awareness was added) stays origin-blind: it always overwrites,
///   exactly as it did when this type had no origin concept at all — safe
///   regardless of what an external crate does with it, since `insert` is
///   the only origin-tagging entry point external code can reach (
///   [`Self::insert_with_origin`] is `pub(crate)`) and every entry it
///   creates is tagged the same fixed `Origin::Author`, so the two-origin
///   precedence rule above never actually branches for external callers.
///   In-crate, the only callers are [`Self::from_source`] and this module's
///   own unit tests, none of which mix origins either.
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct CounterStyleRegistry {
    rules: HashMap<SmolStr, (Origin, CounterStyleRule)>,
}

impl CounterStyleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` and insert every valid `@counter-style` rule it
    /// contains (later same-name rules replace earlier ones — see
    /// [`Self::insert`]'s doc).
    pub fn from_source(source: &str) -> Self {
        let mut registry = Self::new();
        for rule in parse_counter_style_rules(source) {
            registry.insert(rule);
        }
        registry
    }

    /// Insert `rule`, replacing any existing rule of the same name entirely,
    /// **regardless of that existing entry's origin** — this method does not
    /// participate in the [`Origin`]-aware precedence
    /// [`Self::insert_with_origin`] implements (type doc). A rule that fails
    /// [`CounterStyleRule::is_valid`] is silently dropped (does not replace
    /// an existing valid rule of the same name) — this is the one
    /// enforcement point for "only valid rules live in a registry", covering
    /// both the [`parse_counter_style_rules`] entry path and direct
    /// construction of a [`CounterStyleRule`] via its `pub` fields.
    pub fn insert(&mut self, rule: CounterStyleRule) {
        if rule.is_valid() {
            self.rules.insert(rule.name.clone(), (Origin::Author, rule));
        }
    }

    /// Insert `rule` as having come from `origin`, applying CSS Counter
    /// Styles L3 §3's "standard cascade rules" same-name precedence against
    /// whatever is currently stored for `rule.name` (type doc for the exact
    /// resolution table). `pub(crate)` — the one production caller is
    /// [`crate::ruletree::RuleTree::add_stylesheet`]; external crates only
    /// ever reach [`Self::insert`] (origin-blind) or [`Self::from_source`].
    /// A rule that fails [`CounterStyleRule::is_valid`] is silently dropped,
    /// same enforcement point as [`Self::insert`].
    pub(crate) fn insert_with_origin(&mut self, rule: CounterStyleRule, origin: Origin) {
        if !rule.is_valid() {
            return;
        }
        if let Some((existing_origin, _)) = self.rules.get(&rule.name)
            && cascade_rank(origin, false) < cascade_rank(*existing_origin, false)
        {
            return; // Lower-ranked origin never overwrites a higher-ranked one.
        }
        self.rules.insert(rule.name.clone(), (origin, rule));
    }

    pub fn get(&self, name: &str) -> Option<&CounterStyleRule> {
        self.rules.get(name).map(|(_, rule)| rule)
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

// ---------------------------------------------------------------------------
// generate a counter (§2) — the per-system algorithms + resolve_custom_counter
// ---------------------------------------------------------------------------

fn cyclic_repr(symbols: &[CounterSymbol], value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    if n == 0 {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥1.
    }
    let index = (value - 1).rem_euclid(n);
    Some(symbols[index as usize].as_str().to_string())
}

/// CSS Counter Styles L3 §3.1.2
/// <https://www.w3.org/TR/css-counter-styles-3/#fixed-system> verbatim:
/// "Once the list of counter symbols is exhausted, further values cannot be
/// represented by this counter style, and must instead be represented by
/// the fallback counter style." — modeled as `None` here;
/// [`generate_counter`] routes a `None` from any per-system repr function
/// through the same `fallback` hop as an out-of-range value (step 2).
fn fixed_repr(symbols: &[CounterSymbol], first: i64, value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    let index = value - first;
    if index < 0 || index >= n {
        return None;
    }
    Some(symbols[index as usize].as_str().to_string())
}

/// CSS Counter Styles L3 §3.1.5
/// <https://www.w3.org/TR/css-counter-styles-3/#numeric-system> verbatim:
/// "If value is 0, append symbol(0) to S and return S. While value is not
/// equal to 0: Prepend symbol(value mod N) to S. Set value to
/// floor(value / N). Return S." `value` here is already non-negative — it
/// arrives post the negative-sign absolute-value step (numeric "uses a
/// negative sign", see [`system_uses_negative_sign`]).
fn numeric_repr(symbols: &[CounterSymbol], value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    if n < 2 {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥2.
    }
    if value == 0 {
        return Some(symbols[0].as_str().to_string());
    }
    let mut v = value;
    let mut parts: Vec<&str> = Vec::new();
    while v != 0 {
        let digit = v.rem_euclid(n);
        parts.push(symbols[digit as usize].as_str());
        v = v.div_euclid(n);
    }
    parts.reverse();
    Some(parts.concat())
}

/// CSS Counter Styles L3 §3.1.4
/// <https://www.w3.org/TR/css-counter-styles-3/#alphabetic-system>
/// verbatim: "While value is not equal to 0: Set value to value - 1. Prepend
/// symbol(value mod N) to S. Set value to floor(value / N). Finally, return
/// S." Same non-negative-on-entry note as [`numeric_repr`] applies
/// (alphabetic also uses a negative sign).
fn alphabetic_repr(symbols: &[CounterSymbol], value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    if n < 2 {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥2.
    }
    let mut v = value;
    let mut parts: Vec<&str> = Vec::new();
    while v != 0 {
        v -= 1;
        let digit = v.rem_euclid(n);
        parts.push(symbols[digit as usize].as_str());
        v = v.div_euclid(n);
    }
    parts.reverse();
    Some(parts.concat())
}

/// CSS Counter Styles L3 §3.1.3
/// <https://www.w3.org/TR/css-counter-styles-3/#symbolic-system> verbatim:
/// "Let the chosen symbol be symbol((value - 1) mod N). Let the
/// representation length be ceil(value / N). Append the chosen symbol to S a
/// number of times equal to the representation length."
fn symbolic_repr(symbols: &[CounterSymbol], value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    if n == 0 {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥1.
    }
    let index = (value - 1).rem_euclid(n);
    let symbol = symbols[index as usize].as_str();
    let reps = if value <= 0 { 0 } else { (value + n - 1) / n };
    Some(symbol.repeat(reps as usize))
}

/// CSS Counter Styles L3 §3.1.6
/// <https://www.w3.org/TR/css-counter-styles-3/#additive-system> verbatim
/// (4-step numbered algorithm): "(1) Let value initially be the counter
/// value, S initially be the empty string, and symbol list initially be the
/// list of additive tuples. (2) If value is zero: If symbol list contains a
/// tuple with a weight of zero, append that tuple's counter symbol to S and
/// return S. Otherwise, the given counter value cannot be represented by
/// this counter style, and must instead be represented by the fallback
/// counter style. (3) For each tuple in symbol list: Let symbol and weight
/// be tuple's counter symbol and weight, respectively. If weight is zero, or
/// weight is greater than value, continue. Let reps be floor(value /
/// weight). Append symbol to S reps times. Decrement value by weight * reps.
/// If value is zero, return S. (4) Assertion: value is still non-zero. The
/// given counter value cannot be represented by this counter style, and must
/// instead be represented by the fallback counter style."
///
/// Steps 2's and 4's "must instead be represented by the fallback counter
/// style" are both modeled as `None` — same convention as [`fixed_repr`].
fn additive_repr(tuples: &[(i32, CounterSymbol)], value: i64) -> Option<String> {
    if tuples.is_empty() {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥1.
    }
    if value == 0 {
        return tuples
            .iter()
            .find(|(w, _)| *w == 0)
            .map(|(_, sym)| sym.as_str().to_string());
    }
    let mut v = value;
    let mut s = String::new();
    for (weight, symbol) in tuples {
        let w = i64::from(*weight);
        if w == 0 || w > v {
            continue;
        }
        let reps = v / w;
        for _ in 0..reps {
            s.push_str(symbol.as_str());
        }
        v -= w * reps;
        if v == 0 {
            return Some(s);
        }
    }
    None
}

/// Resolve `name` against `registry` and format `value` using CSS Counter
/// Styles L3's `generate a counter` algorithm — §2 "Counter Styles"
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-styles> verbatim
/// (6-step algorithm): "(1) If the counter style is unknown, exit this
/// algorithm and instead generate a counter representation using the
/// decimal style and the same counter value. (2) If the counter value is
/// outside the range of the counter style, exit this algorithm and instead
/// generate a counter representation using the counter style's fallback
/// style and the same counter value. (3) Using the counter value and the
/// counter algorithm for the counter style, generate an initial
/// representation for the counter value. If the counter value is negative
/// and the counter style uses a negative sign, instead generate an initial
/// representation using the absolute value of the counter value. (4)
/// Prepend symbols to the representation as specified in the pad
/// descriptor. (5) If the counter value is negative and the counter style
/// uses a negative sign, wrap the representation in the counter style's
/// negative sign as specified in the negative descriptor. (6) Return the
/// representation."
///
/// Note what step 6's "the representation" is: this 6-step algorithm has no
/// prefix/suffix step, because those only apply to the `::marker`
/// pseudo-element's default contents, not to `counter()`/`counters()` (§3.3
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-prefix>, §3.4
/// <https://www.w3.org/TR/css-counter-styles-3/#counter-style-suffix>). This
/// function returns exactly that `counter()`-context representation — see
/// [`CounterStyleRule::prefix`] for the full citation on why those two
/// fields are parsed and stored but never consulted here.
///
/// # Contract
///
/// - `Some(_)` — `name` (or something it transitively falls back to, within
///   `registry`) resolved to a concrete representation.
/// - `None` — any of: `name` isn't in `registry` (step 1's "unknown" case);
///   the fallback chain bottoms out at a name that also isn't in `registry`
///   (almost always `decimal`, the spec default `fallback` value — see the
///   crate-boundary note below); a fallback cycle was detected — CSS Counter
///   Styles L3 §3.7 "fallback"
///   (<https://www.w3.org/TR/css-counter-styles-3/#counter-style-fallback>)
///   verbatim: "while following fallbacks to find a counter style that can
///   render the given counter value, if a loop in the specified fallbacks is
///   detected, the decimal style must be used instead" — i.e. this is
///   spec-*mandated* loop detection, not a robustness guard this crate
///   invented on top of the spec (matching this function's own "defer
///   `decimal` to the caller via `None`" boundary is what makes returning
///   `None` here spec-correct rather than merely defensive); or the
///   winning rule's `system` is `extends` (composition intentionally
///   unimplemented, see [`CounterStyleSystem::Extends`]).
///
/// **Why an unresolvable name returns `None` instead of formatting
/// `decimal` (or another CSS-Counter-Styles-Level-3-*predefined* style)
/// itself**: `raikiri-style` cannot depend on `raikiri-traits` — the
/// dependency direction is the other way (`raikiri-traits/src/page/target.rs`
/// already does `use raikiri_style::property::{..., CounterStyle}`) — and
/// the predefined-style formatting logic (`decimal-leading-zero` /
/// `*-roman` / `*-alpha` / `*-latin` / `disc` / `circle` / `square`) already
/// lives there, in `format_counter` / `format_named_counter`. Reimplementing
/// it here would create exactly the kind
/// of duplicated-source-of-truth drift this project has hit before (see
/// e.g. [`crate::page::PageCascadeResult::declarations`]'s doc for a
/// repeated instance of the same drift). `None` is this
/// function's honest boundary: it is `raikiri-traits::format_counter`
/// (today unconditionally decimal-formatting any `CounterStyle::Named` it
/// doesn't itself recognize) that is expected to treat `None` from this
/// function the same way. Actually wiring that call is a deferred design
/// question — not this function's job.
pub fn resolve_custom_counter(
    registry: &CounterStyleRegistry,
    name: &str,
    value: i32,
) -> Option<String> {
    let mut visited: Vec<SmolStr> = Vec::new();
    generate_counter(registry, name, value, &mut visited)
}

fn generate_counter(
    registry: &CounterStyleRegistry,
    name: &str,
    value: i32,
    visited: &mut Vec<SmolStr>,
) -> Option<String> {
    // Step 1.
    let rule = registry.get(name)?;
    // Per §3.7's explicit loop-detection rule ("if a loop in the specified
    // fallbacks is detected, the decimal style must be used instead" — full
    // quote + anchor on this function's doc), modeled here via the crate's
    // `None`-defers-to-`decimal` boundary convention: refuse to loop through
    // a `fallback` chain that revisits a name.
    if visited.iter().any(|v| v.as_str() == rule.name.as_str()) {
        return None;
    }
    visited.push(rule.name.clone());

    // Step 2.
    if !rule_contains_value(rule, value) {
        return generate_counter(registry, rule.fallback.as_str(), value, visited);
    }

    // Step 3 — the negative-sign absolute-value adjustment applies uniformly
    // here: for a system that doesn't use a negative sign (cyclic / fixed /
    // the moot `extends` case), `system_uses_negative_sign` is `false` and
    // `algorithm_value` is just `value` unchanged.
    let uses_negative = system_uses_negative_sign(&rule.system);
    let value_i64 = i64::from(value);
    let algorithm_value = if uses_negative && value_i64 < 0 {
        -value_i64
    } else {
        value_i64
    };
    let initial = match &rule.system {
        CounterStyleSystem::Cyclic => cyclic_repr(&rule.symbols, algorithm_value),
        CounterStyleSystem::Fixed {
            first_symbol_value: first,
        } => fixed_repr(&rule.symbols, i64::from(*first), algorithm_value),
        CounterStyleSystem::Numeric => numeric_repr(&rule.symbols, algorithm_value),
        CounterStyleSystem::Alphabetic => alphabetic_repr(&rule.symbols, algorithm_value),
        CounterStyleSystem::Symbolic => symbolic_repr(&rule.symbols, algorithm_value),
        CounterStyleSystem::Additive => additive_repr(&rule.additive_symbols, algorithm_value),
        // Scope-cut — see `CounterStyleSystem::Extends`'s doc.
        CounterStyleSystem::Extends(_) => None,
    };
    let Some(mut repr) = initial else {
        return generate_counter(registry, rule.fallback.as_str(), value, visited);
    };

    // Step 4 — §3.6's pad algorithm (quoted in full on `apply_pad`) reduces
    // its padding `difference` by the negative descriptor's own length
    // *before* step 5 wraps it on, so the negative sign's presence still
    // counts toward `pad.min_length` even though it isn't part of `repr`
    // yet at this point in the algorithm.
    let negative_reserved = if uses_negative && value_i64 < 0 {
        negative_descriptor_len(&rule.negative)
    } else {
        0
    };
    repr = apply_pad(&rule.pad, repr, negative_reserved);

    // Step 5.
    if uses_negative && value_i64 < 0 {
        let mut wrapped = String::new();
        wrapped.push_str(rule.negative.prefix.as_str());
        wrapped.push_str(&repr);
        if let Some(suffix) = &rule.negative.suffix {
            wrapped.push_str(suffix.as_str());
        }
        repr = wrapped;
    }

    // Step 6.
    Some(repr)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parsing: rule name / prelude ──────────────────────────────────

    #[test]
    fn minimal_cyclic_rule_parses() {
        let rules =
            parse_counter_style_rules(r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name.as_str(), "thumbs");
        assert_eq!(rules[0].system, CounterStyleSystem::Cyclic);
        assert_eq!(rules[0].symbols, vec![CounterSymbol(SmolStr::new("*"))]);
    }

    #[test]
    fn defaulted_system_is_symbolic() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].system, CounterStyleSystem::Symbolic);
    }

    #[test]
    fn reserved_rule_names_are_dropped() {
        for name in [
            "decimal",
            "disc",
            "circle",
            "square",
            "disclosure-open",
            "disclosure-closed",
            "none",
            "DECIMAL",
        ] {
            let src = format!(r#"@counter-style {name} {{ system: cyclic; symbols: "*"; }}"#);
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes when the assertion fails; every iteration here
            // passes, so llvm-cov reports the macro's condition-false region
            // as an uncovered added line (attributed to the `assert!(`
            // line) even though the assertion itself runs, and does its
            // job, on every iteration.
            assert!(
                parse_counter_style_rules(&src).is_empty(),
                "expected {name:?} to be rejected as a rule name"
            );
        }
    }

    #[test]
    fn reserved_names_are_still_valid_as_fallback_references() {
        // §3 dfn: those keywords "are valid <counter-style-name>s, but are
        // invalid when used here to name a counter style rule" — i.e. valid
        // as a *reference*.
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: cyclic; symbols: "*"; fallback: disc; }"#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].fallback.as_str(), "disc");
    }

    #[test]
    fn non_counter_style_at_rules_and_qualified_rules_are_ignored() {
        let rules = parse_counter_style_rules(
            r#"
            p { color: red }
            @media print { p { color: blue } }
            @counter-style thumbs { system: cyclic; symbols: "*"; }
            "#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name.as_str(), "thumbs");
    }

    #[test]
    fn empty_source_produces_no_rules() {
        assert!(parse_counter_style_rules("").is_empty());
    }

    // ── Parsing: individual descriptors ───────────────────────────────

    #[test]
    fn system_extends_stores_the_extended_name() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { system: extends decimal; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].system,
            CounterStyleSystem::Extends(SmolStr::new("decimal"))
        );
    }

    #[test]
    fn system_fixed_default_first_value_is_one() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { system: fixed; symbols: "a" "b"; }"#);
        assert_eq!(
            rules[0].system,
            CounterStyleSystem::Fixed {
                first_symbol_value: 1
            }
        );
    }

    #[test]
    fn system_fixed_explicit_first_value() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { system: fixed 5; symbols: "a"; }"#);
        assert_eq!(
            rules[0].system,
            CounterStyleSystem::Fixed {
                first_symbol_value: 5
            }
        );
    }

    #[test]
    fn system_unknown_keyword_drops_declaration_system_stays_default() {
        // `bogus` matches none of the 7 `system` keywords -> `parse_system`
        // returns `None` -> the whole `system` declaration is dropped (this
        // crate's usual invalid-declaration handling), leaving `system` at
        // its spec-initial value `symbolic`. `symbols: "*"` (1 entry) is
        // enough for `symbolic`'s own validity minimum, so the rest of the
        // rule still parses.
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { system: bogus; symbols: "*"; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].system, CounterStyleSystem::Symbolic);
    }

    #[test]
    fn negative_descriptor_prefix_only() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: numeric; symbols: "0" "1"; negative: "neg-"; }"#,
        );
        assert_eq!(
            rules[0].negative,
            NegativeDescriptor {
                prefix: CounterSymbol(SmolStr::new("neg-")),
                suffix: None,
            }
        );
    }

    #[test]
    fn negative_descriptor_prefix_and_suffix() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: numeric; symbols: "0" "1"; negative: "(" ")"; }"#,
        );
        assert_eq!(
            rules[0].negative,
            NegativeDescriptor {
                prefix: CounterSymbol(SmolStr::new("(")),
                suffix: Some(CounterSymbol(SmolStr::new(")"))),
            }
        );
    }

    #[test]
    fn negative_descriptor_default_is_hyphen_minus_only() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
        assert_eq!(rules[0].negative, NegativeDescriptor::default());
        assert_eq!(rules[0].negative.prefix.as_str(), "-");
    }

    #[test]
    fn prefix_and_suffix_descriptors() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { symbols: "*"; prefix: "["; suffix: "]"; }"#,
        );
        assert_eq!(rules[0].prefix, CounterSymbol(SmolStr::new("[")));
        assert_eq!(rules[0].suffix, CounterSymbol(SmolStr::new("]")));
    }

    #[test]
    fn default_prefix_suffix_match_spec_initial_values() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
        assert_eq!(rules[0].prefix.as_str(), "");
        assert_eq!(rules[0].suffix.as_str(), ". ");
    }

    #[test]
    fn pad_integer_then_symbol_order() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: 3 "0"; }"#);
        assert_eq!(
            rules[0].pad,
            PadDescriptor {
                min_length: 3,
                symbol: CounterSymbol(SmolStr::new("0")),
            }
        );
    }

    #[test]
    fn pad_symbol_then_integer_order() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: "0" 3; }"#);
        assert_eq!(
            rules[0].pad,
            PadDescriptor {
                min_length: 3,
                symbol: CounterSymbol(SmolStr::new("0")),
            }
        );
    }

    #[test]
    fn pad_negative_integer_is_rejected() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: -1 "0"; }"#);
        // Whole `pad` declaration dropped, rest of the rule survives with
        // pad at its default.
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pad, PadDescriptor::default());
    }

    #[test]
    fn pad_negative_integer_is_rejected_in_symbol_first_order_too() {
        // Sibling of `pad_negative_integer_is_rejected` above, but for the
        // *other* `&&` order (`<symbol> <integer>` instead of `<integer>
        // <symbol>`) — `parse_nonneg_int_and_symbol`'s second branch.
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: "0" -1; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pad, PadDescriptor::default());
    }

    #[test]
    fn range_single_pair() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: 1 5; }"#);
        assert_eq!(
            rules[0].range,
            CounterRange::List(vec![RangeEntry {
                lower: RangeLimit::Finite(1),
                upper: RangeLimit::Finite(5),
            }])
        );
    }

    #[test]
    fn range_comma_separated_multiple_pairs() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { symbols: "*"; range: 1 5, 10 infinite; }"#,
        );
        assert_eq!(
            rules[0].range,
            CounterRange::List(vec![
                RangeEntry {
                    lower: RangeLimit::Finite(1),
                    upper: RangeLimit::Finite(5),
                },
                RangeEntry {
                    lower: RangeLimit::Finite(10),
                    upper: RangeLimit::Infinite,
                },
            ])
        );
    }

    #[test]
    fn range_lower_greater_than_upper_reverts_to_auto() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: 5 1; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].range, CounterRange::Auto);
    }

    #[test]
    fn range_auto_keyword_is_explicit_default() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: auto; }"#);
        assert_eq!(rules[0].range, CounterRange::Auto);
    }

    #[test]
    fn fallback_default_is_decimal() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
        assert_eq!(rules[0].fallback.as_str(), "decimal");
    }

    #[test]
    fn fallback_custom_name() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { symbols: "*"; fallback: my-other-style; }"#,
        );
        assert_eq!(rules[0].fallback.as_str(), "my-other-style");
    }

    #[test]
    fn fallback_none_is_rejected_declaration_dropped() {
        // `<counter-style-name>` (`parse_counter_style_name_ref`) excludes
        // `none` even in *reference* position (unlike the 6 predefined-style
        // keywords, which are valid references — see
        // `reserved_names_are_still_valid_as_fallback_references` above).
        // The whole `fallback` declaration is dropped, leaving `fallback` at
        // its spec-initial value `decimal`.
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; fallback: none; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].fallback.as_str(), "decimal");
    }

    #[test]
    fn additive_symbols_descending_weight_parses() {
        let rules = parse_counter_style_rules(
            r#"@counter-style roman-ish { system: additive; additive-symbols: 10 "X", 5 "V", 1 "I"; }"#,
        );
        assert_eq!(
            rules[0].additive_symbols,
            vec![
                (10, CounterSymbol(SmolStr::new("X"))),
                (5, CounterSymbol(SmolStr::new("V"))),
                (1, CounterSymbol(SmolStr::new("I"))),
            ]
        );
    }

    #[test]
    fn additive_symbols_non_descending_order_invalidates_whole_rule() {
        // system: additive requires additive_symbols non-empty (`is_valid`);
        // a non-descending order drops the *declaration* (reverts to empty),
        // which then fails whole-rule validity for `additive`.
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: additive; additive-symbols: 1 "I", 5 "V"; }"#,
        );
        assert!(rules.is_empty());
    }

    #[test]
    fn additive_without_additive_symbols_is_whole_rule_invalid() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { system: additive; }"#);
        assert!(rules.is_empty());
    }

    #[test]
    fn extends_forbids_symbols_whole_rule_invalid() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: extends decimal; symbols: "*"; }"#,
        );
        assert!(rules.is_empty());
    }

    #[test]
    fn extends_forbids_additive_symbols_whole_rule_invalid() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { system: extends decimal; additive-symbols: 1 "I"; }"#,
        );
        assert!(rules.is_empty());
    }

    #[test]
    fn extends_alone_is_valid() {
        let rules = parse_counter_style_rules(r#"@counter-style foo { system: extends decimal; }"#);
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn cyclic_requires_at_least_one_symbol() {
        assert!(parse_counter_style_rules(r#"@counter-style foo { system: cyclic; }"#).is_empty());
    }

    #[test]
    fn numeric_requires_at_least_two_symbols() {
        assert!(
            parse_counter_style_rules(r#"@counter-style foo { system: numeric; symbols: "0"; }"#)
                .is_empty()
        );
        assert_eq!(
            parse_counter_style_rules(
                r#"@counter-style foo { system: numeric; symbols: "0" "1"; }"#
            )
            .len(),
            1
        );
    }

    #[test]
    fn alphabetic_requires_at_least_two_symbols() {
        assert!(
            parse_counter_style_rules(
                r#"@counter-style foo { system: alphabetic; symbols: "a"; }"#
            )
            .is_empty()
        );
    }

    #[test]
    fn unsupported_descriptor_speak_as_is_silently_dropped() {
        let rules =
            parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; speak-as: numbers; }"#);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].symbols, vec![CounterSymbol(SmolStr::new("*"))]);
    }

    #[test]
    fn later_declaration_of_same_descriptor_wins() {
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { prefix: "a"; prefix: "b"; symbols: "*"; }"#,
        );
        assert_eq!(rules[0].prefix, CounterSymbol(SmolStr::new("b")));
    }

    #[test]
    fn trailing_garbage_after_descriptor_value_drops_declaration() {
        // `pad`'s grammar is exactly `<integer> && <symbol>` (no
        // repetition), so a 3rd token after both components is genuine
        // leftover — `CounterStyleDeclParser::parse_value`'s
        // `expect_exhausted()` call rejects the whole declaration rather
        // than silently accepting the `3 "0"` prefix. `pad` stays at its
        // default; the rest of the rule survives.
        let rules = parse_counter_style_rules(
            r#"@counter-style foo { symbols: "*"; pad: 3 "0" garbage; }"#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pad, PadDescriptor::default());
    }

    // ── Registry ───────────────────────────────────────────────────────

    #[test]
    fn registry_from_source_and_get() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        );
        assert_eq!(registry.len(), 1);
        assert!(registry.get("thumbs").is_some());
        assert!(registry.get("unknown").is_none());
    }

    #[test]
    fn registry_empty_is_empty() {
        let registry = CounterStyleRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn later_same_name_rule_replaces_entirely() {
        let registry = CounterStyleRegistry::from_source(
            r#"
            @counter-style foo { system: cyclic; symbols: "a"; prefix: "["; }
            @counter-style foo { system: numeric; symbols: "0" "1"; }
            "#,
        );
        assert_eq!(registry.len(), 1);
        let rule = registry.get("foo").unwrap();
        assert_eq!(rule.system, CounterStyleSystem::Numeric);
        // The first rule's `prefix: "["` did NOT survive — atomic replace,
        // not a per-descriptor merge.
        assert_eq!(rule.prefix.as_str(), "");
    }

    #[test]
    fn insert_rejects_invalid_rule_constructed_directly() {
        let mut registry = CounterStyleRegistry::new();
        let mut invalid = CounterStyleRule::new(SmolStr::new("foo"));
        invalid.system = CounterStyleSystem::Additive; // additive_symbols left empty -> invalid
        registry.insert(invalid);
        assert!(registry.is_empty());
    }

    #[test]
    fn insert_with_origin_rejects_invalid_rule_constructed_directly() {
        // insert_with_origin has its own is_valid
        // gate, same enforcement point as insert() above — but every
        // production caller (RuleTree::add_stylesheet) only ever feeds it
        // rules that already passed parse_counter_style_rules's own
        // is_valid filter, so that gate is otherwise unreachable through
        // add_stylesheet. Exercise it directly, the same way
        // insert_rejects_invalid_rule_constructed_directly does for
        // insert(). Origin::UserAgent here is an arbitrary choice — the
        // is_valid check is origin-independent, it runs before the
        // origin-precedence resolution.
        let mut registry = CounterStyleRegistry::new();
        let mut invalid = CounterStyleRule::new(SmolStr::new("foo"));
        invalid.system = CounterStyleSystem::Additive; // additive_symbols left empty -> invalid
        registry.insert_with_origin(invalid, Origin::UserAgent);
        assert!(registry.is_empty());
    }

    // ── resolve_custom_counter: cyclic ────────────────────────────────

    #[test]
    fn resolve_cyclic_wraps_through_symbols() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", 1).as_deref(),
            Some("A")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", 2).as_deref(),
            Some("B")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", 3).as_deref(),
            Some("C")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", 4).as_deref(),
            Some("A")
        );
    }

    #[test]
    fn resolve_cyclic_handles_zero_and_negative_without_negative_sign() {
        // Cyclic never uses a negative sign — negative values just keep
        // cycling per `(value-1) mod N`, no `-` wrapping.
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", 0).as_deref(),
            Some("C")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", -1).as_deref(),
            Some("B")
        );
    }

    // ── resolve_custom_counter: numeric ───────────────────────────────

    #[test]
    fn resolve_numeric_base_three() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "base3", 0).as_deref(),
            Some("0")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "base3", 5).as_deref(),
            Some("12")
        );
    }

    #[test]
    fn resolve_numeric_negative_wraps_with_default_negative_sign() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "base3", -5).as_deref(),
            Some("-12")
        );
    }

    #[test]
    fn resolve_numeric_negative_with_custom_negative_descriptor() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; negative: "(" ")"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "base3", -5).as_deref(),
            Some("(12)")
        );
    }

    // ── resolve_custom_counter: alphabetic ────────────────────────────

    #[test]
    fn resolve_alphabetic_bijective_base26() {
        let symbols = ('a'..='z')
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(" ");
        let src = format!(r#"@counter-style latin {{ system: alphabetic; symbols: {symbols}; }}"#);
        let registry = CounterStyleRegistry::from_source(&src);
        assert_eq!(
            resolve_custom_counter(&registry, "latin", 1).as_deref(),
            Some("a")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "latin", 26).as_deref(),
            Some("z")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "latin", 27).as_deref(),
            Some("aa")
        );
    }

    // ── resolve_custom_counter: symbolic ──────────────────────────────

    #[test]
    fn resolve_symbolic_doubles_symbol_on_successive_passes() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style stars { system: symbolic; symbols: "*"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "stars", 1).as_deref(),
            Some("*")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "stars", 2).as_deref(),
            Some("**")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "stars", 3).as_deref(),
            Some("***")
        );
    }

    // ── resolve_custom_counter: fixed ─────────────────────────────────

    #[test]
    fn resolve_fixed_within_window() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style small { system: fixed 1; symbols: "one" "two" "three"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "small", 1).as_deref(),
            Some("one")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "small", 3).as_deref(),
            Some("three")
        );
    }

    #[test]
    fn resolve_fixed_exhausted_falls_back_to_unresolvable_decimal_default() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style small { system: fixed 1; symbols: "one" "two" "three"; }"#,
        );
        // fallback defaults to "decimal", which is not itself in this
        // registry — see `resolve_custom_counter`'s doc on why that's `None`
        // rather than this crate formatting "4" itself.
        assert_eq!(resolve_custom_counter(&registry, "small", 4), None);
    }

    #[test]
    fn resolve_fixed_exhausted_falls_back_to_custom_style_in_registry() {
        let registry = CounterStyleRegistry::from_source(
            r#"
            @counter-style small { system: fixed 1; symbols: "one" "two"; fallback: overflow; }
            @counter-style overflow { system: cyclic; symbols: "%"; }
            "#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "small", 5).as_deref(),
            Some("%")
        );
    }

    // ── resolve_custom_counter: additive ──────────────────────────────

    #[test]
    fn resolve_additive_roman_like() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style toy-roman { system: additive; additive-symbols: 10 "X", 5 "V", 1 "I"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "toy-roman", 7).as_deref(),
            Some("VII")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "toy-roman", 11).as_deref(),
            Some("XI")
        );
    }

    #[test]
    fn resolve_additive_unrepresentable_value_falls_back() {
        // No tuple/combination reaches exactly 0 for value 2 given only a
        // weight-1 "I" as the smallest tuple would actually make 2
        // representable ("II") — use a registry where the smallest weight
        // is too large to reach 0 for the probed value.
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style big-only { system: additive; additive-symbols: 100 "C"; }"#,
        );
        assert_eq!(resolve_custom_counter(&registry, "big-only", 50), None);
    }

    #[test]
    fn resolve_additive_zero_weight_tuple_handles_value_zero() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style with-zero { system: additive; additive-symbols: 1 "I", 0 "Z"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "with-zero", 0).as_deref(),
            Some("Z")
        );
    }

    // ── resolve_custom_counter: range / pad ───────────────────────────

    #[test]
    fn resolve_custom_range_out_of_bounds_falls_back() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style limited { system: cyclic; symbols: "*"; range: 1 3; }"#,
        );
        assert_eq!(resolve_custom_counter(&registry, "limited", 4), None);
        assert_eq!(
            resolve_custom_counter(&registry, "limited", 2).as_deref(),
            Some("*")
        );
    }

    #[test]
    fn resolve_pad_prepends_symbol_to_minimum_length() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "padded", 5).as_deref(),
            Some("005")
        );
        assert_eq!(
            resolve_custom_counter(&registry, "padded", 500).as_deref(),
            Some("500")
        );
    }

    /// §3.6's own worked example (verbatim quote on `apply_pad`): the pad
    /// `difference` is reduced by the negative descriptor's own length
    /// *before* padding, so a negative value's sign counts toward
    /// `pad.min_length` — `pad: 3 "0"` + value `-5` (default `negative: "-"`,
    /// length 1) must be `"-05"`, not `"-005"`.
    #[test]
    fn resolve_pad_reduces_difference_by_default_negative_sign_length() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "padded", -5).as_deref(),
            Some("-05")
        );
    }

    /// Same rule as the sibling test above, but with a 2-symbol `negative`
    /// descriptor (`"(" ")"`, combined length 2) — `pad: 3 "0"` + value `-5`
    /// has `difference = 3 - 1 - 2 = 0`, so no padding is prepended at all
    /// and the result is `"(5)"`, not `"(005)"`.
    #[test]
    fn resolve_pad_reduces_difference_by_two_symbol_negative_descriptor_length() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; negative: "(" ")"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "padded", -5).as_deref(),
            Some("(5)")
        );
    }

    /// Padding for a *positive* value must not be affected by the negative
    /// descriptor's length — `negative_reserved` is `0` whenever the value
    /// isn't negative, per §3.6's own gating clause.
    #[test]
    fn resolve_pad_unaffected_by_negative_descriptor_for_positive_value() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; negative: "(" ")"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "padded", 5).as_deref(),
            Some("005")
        );
    }

    /// The `generate_counter` step-4 gate (`uses_negative && value_i64 < 0`,
    /// see its doc) is also false when the *value* is negative but the
    /// *system* doesn't use a negative sign (`cyclic` here) — not just when
    /// the value is non-negative (the sibling test above). `negative_reserved`
    /// must still be `0` in that case, so the pad `difference` is computed
    /// from `repr`'s length alone: `cyclic` with 3 symbols and value `-5`
    /// produces the single symbol `"A"` (`(-5-1).rem_euclid(3) == 0`), and
    /// `pad: 3 "0"` pads it to `"00A"` — the 2-symbol `negative: "(" ")"`
    /// descriptor (length 2) must NOT be subtracted from the difference, and
    /// step 5's negative-sign wrapping must not apply either.
    #[test]
    fn resolve_pad_unaffected_by_negative_descriptor_for_negative_value_without_negative_sign() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; pad: 3 "0"; negative: "(" ")"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "thumbs", -5).as_deref(),
            Some("00A")
        );
    }

    // ── resolve_custom_counter: fixed auto-range is unbounded (§3.5) ──

    /// §3.5 verbatim: "For cyclic, numeric, and fixed systems, the range is
    /// negative infinity to positive infinity." — `fixed`'s actual
    /// window-exhaustion behavior is enforced independently by `fixed_repr`
    /// (see `auto_range`'s doc), so a `first_symbol_value` near `i32::MAX`
    /// must not make `auto_range` compute an inverted (`lower > upper`)
    /// finite window that silently rejects every value via the range check
    /// at step 2 before `fixed_repr` ever runs.
    #[test]
    fn resolve_fixed_near_i32_max_does_not_invert_range() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style edge { system: fixed 2147483647; symbols: "Z"; }"#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "edge", i32::MAX).as_deref(),
            Some("Z")
        );
    }

    // ── resolve_custom_counter: extends scope-cut ─────────────────────

    #[test]
    fn resolve_extends_is_unimplemented_and_falls_through() {
        let registry = CounterStyleRegistry::from_source(
            r#"@counter-style my-decimal-ish { system: extends decimal; }"#,
        );
        // fallback defaults to "decimal", not in this registry -> None.
        assert_eq!(resolve_custom_counter(&registry, "my-decimal-ish", 5), None);
    }

    #[test]
    fn resolve_extends_falls_back_to_custom_style_when_declared() {
        let registry = CounterStyleRegistry::from_source(
            r#"
            @counter-style my-decimal-ish { system: extends decimal; fallback: real; }
            @counter-style real { system: cyclic; symbols: "R"; }
            "#,
        );
        assert_eq!(
            resolve_custom_counter(&registry, "my-decimal-ish", 5).as_deref(),
            Some("R")
        );
    }

    // ── resolve_custom_counter: fallback chains / unknown / cycles ────

    #[test]
    fn resolve_unknown_name_is_none() {
        let registry = CounterStyleRegistry::new();
        assert_eq!(resolve_custom_counter(&registry, "nonexistent", 1), None);
    }

    #[test]
    fn resolve_chains_through_two_custom_fallbacks() {
        let registry = CounterStyleRegistry::from_source(
            r#"
            @counter-style a { system: cyclic; symbols: "*"; range: 1 1; fallback: b; }
            @counter-style b { system: cyclic; symbols: "%"; range: 1 1; fallback: c; }
            @counter-style c { system: cyclic; symbols: "@"; }
            "#,
        );
        // value 2 is out of `a`'s range -> falls to b -> also out of range -> falls to c.
        assert_eq!(
            resolve_custom_counter(&registry, "a", 2).as_deref(),
            Some("@")
        );
    }

    #[test]
    fn resolve_fallback_cycle_terminates_with_none() {
        let registry = CounterStyleRegistry::from_source(
            r#"
            @counter-style a { system: cyclic; symbols: "*"; range: 1 1; fallback: b; }
            @counter-style b { system: cyclic; symbols: "%"; range: 1 1; fallback: a; }
            "#,
        );
        // Neither `a` nor `b` accepts value 5 (both ranged to exactly 1);
        // the fallback chain cycles a -> b -> a -> ... and must terminate.
        assert_eq!(resolve_custom_counter(&registry, "a", 5), None);
    }
}
