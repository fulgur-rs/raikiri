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
//! `generate a counter` algorithm. Wiring formatted custom-counter strings
//! into `raikiri-traits::TargetRegistry`'s target-resolution flow remains
//! deferred; ordinary marker and generated-content paint consumes this module
//! through the cascade result without adding a dependency on
//! `raikiri-traits`. See [`resolve_custom_counter`]'s doc for the exact
//! representation contract.
//!
//! What's implemented:
//!
//! - Full descriptor grammar: `system`,
//!   `negative`, `prefix`, `suffix`, `range`, `pad`, `fallback`, `symbols`,
//!   `additive-symbols`. `speak-as` is deliberately **not** in that list
//!   (audio-rendering concern, no bearing on the text-formatting use case
//!   this module serves) and is silently dropped like any other unsupported
//!   descriptor or at-rule, per this crate's general convention (e.g.
//!   [`crate::ruletree`]'s `@media`/`@supports`, or an unrecognized nested
//!   at-rule inside `@page`).
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
//! # RuleTree integration
//!
//! [`crate::ruletree::RuleTree`] populates its [`CounterStyleRegistry`] while
//! parsing stylesheets, for every stylesheet origin. Same-name rules follow
//! CSS Counter Styles Level 3 §3 cascade ordering, with author rules taking
//! precedence over user-agent rules. The cascade result exposes the registry
//! to marker and generated-content counter resolution through
//! [`resolve_custom_counter`].
//!
//! Target-counter resolution through `raikiri-traits::TargetRegistry` remains
//! outside this module.

use std::collections::{HashMap, HashSet};

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};
use smol_str::SmolStr;

use crate::cascade::cascade_rank;
use crate::property::{is_reserved_custom_ident, parse_custom_ident};
use crate::ruletree::{Origin, ParseBudget};

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
    /// **Composition is not implemented** (scope-limited, following the general
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
/// step 3 dispatch returns `None` unconditionally, per the scope-limited on
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

/// Parse the shared `<integer [0,∞]> && <symbol>` grammar used by `pad`
/// (§3.6) and `additive-symbols` (§3.8). Both components are required and may
/// appear in either order.
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
/// **Scope-cut**: lengths here use Unicode scalar values from `.chars()`, not
/// grapheme clusters. Combining sequences and ZWJ emoji therefore count as
/// multiple units and may be padded more than a grapheme-based implementation.
///
/// `pad.min_length` is parsed straight off an author-supplied CSS
/// `<integer [0,∞]>` with no upper bound (`parse_pad`), so `diff` — and
/// therefore the number of `pad.symbol` copies this function would
/// otherwise unconditionally allocate — is likewise unbounded. Returns
/// `None` rather than padding when the pad addition's own bytes, *combined
/// with `repr`'s own byte length*, would exceed [`MAX_REPR_BYTES`] (see that
/// constant's doc) — not just the pad addition measured in isolation, since
/// `repr` already contributes its own bytes to the representation this
/// function builds. [`generate_counter`] treats `None` here the same as a
/// `None` initial representation — fall through to the rule's fallback
/// counter style (step 4 in its own doc).
fn apply_pad(pad: &PadDescriptor, repr: String, negative_reserved: i64) -> Option<String> {
    let len = repr.chars().count() as i64;
    let diff = i64::from(pad.min_length) - len - negative_reserved;
    if diff > 0 {
        let pad_bytes = checked_repr_bytes(diff, pad.symbol.as_str().len())?;
        // Budget the *combined* output — `repr`'s own bytes plus the pad
        // addition — against `MAX_REPR_BYTES`, not just the pad portion in
        // isolation. `saturating_add` rather than plain `+`: `repr_bytes` and
        // `pad_bytes` are each independently derived (one from an already-
        // built `String`, the other from an author-supplied `min_length`
        // with no upper bound), so their sum could in principle overflow
        // `u64`; a saturated sum is still unambiguously over budget, which
        // is the only thing this comparison needs to know.
        let repr_bytes = repr.len() as u64;
        if repr_bytes.saturating_add(pad_bytes) > MAX_REPR_BYTES {
            return None;
        }
        let mut out = pad.symbol.as_str().repeat(diff as usize);
        out.push_str(&repr);
        Some(out)
    } else {
        Some(repr)
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
/// `fixed` is grouped with the unbounded systems here, as required by §3.5.
/// Its symbol-exhaustion behavior is handled separately by [`fixed_repr`],
/// which falls back when the requested value cannot be represented.
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
///   fallback for an `extends` rule — the scope-limited fallback is always
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

/// A parsed `@counter-style` rule — one entry per descriptor defined by CSS
/// Counter Styles Level 3 §3, excluding `speak-as` (see the module doc).
/// Registry insertion enforces the invariant that only spec-valid rules are
/// retained.
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
    /// oversight. The `raikiri-paint` default-marker consumer reads this field
    /// directly and applies it around the representation returned by
    /// [`resolve_custom_counter`].
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
    /// Invalid rules are rejected by the registry and do not define a
    /// counter style.
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
/// same "later wins" fold [`crate::rule::parse_declaration_block_within`]'s callers
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
/// `RuleBodyItemParser` integration, same "unsupported name / invalid value →
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

/// Parse one descriptor block with the same recovery and validity as the standalone parser.
/// Crate-visible so stylesheet and conditional group parsing share this grammar.
///
/// Each descriptor counts as a declaration against `budget` as it is parsed,
/// so a block past the limit stops there and yields no rule.
pub(crate) fn parse_counter_style_rule(
    name: SmolStr,
    input: &mut Parser<'_, '_>,
    budget: &mut ParseBudget,
) -> Option<CounterStyleRule> {
    let mut parser = CounterStyleDeclParser;
    let mut descriptors = Vec::new();
    for descriptor in RuleBodyParser::new(input, &mut parser).flatten() {
        if !budget.declarations(1) {
            return None;
        }
        descriptors.push(descriptor);
    }
    let rule = build_rule(name, descriptors);
    rule.is_valid().then_some(rule)
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
/// Crate-visible so stylesheet and conditional group parsers share the naming grammar.
pub(crate) fn parse_counter_style_rule_name<'i>(
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
/// [`crate::ruletree`]'s identical `ParsedRule` shape for the same reason.
enum TopLevelItem {
    CounterStyle(SmolStr, Vec<ParsedDescriptor>),
}

/// Standalone stylesheet parser that accepts only `@counter-style` blocks.
/// [`crate::ruletree::RuleTree`] collects individual rules while parsing
/// stylesheets and conditional groups.
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
/// CSS Counter Styles Level 3 §3 requires same-name rules to be selected by
/// the standard cascade and replaced atomically. This registry stores one
/// complete rule per name and records its origin so insertion can apply that
/// precedence while preserving source order within an origin.
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

/// Byte budget for generated counter representations.
///
/// CSS Counter Styles Level 3 §2 warns that `symbolic`, `additive`, and
/// `pad` can otherwise allocate from author-supplied sizes. Values that would
/// exceed this budget return `None` and use the fallback counter style.
const MAX_REPR_BYTES: u64 = 64 * 1024;

/// Return the byte count for `reps * symbol_len`, or `None` on overflow.
/// Callers treat overflow as exceeding [`MAX_REPR_BYTES`].
fn checked_repr_bytes(reps: i64, symbol_len: usize) -> Option<u64> {
    (reps as u64).checked_mul(symbol_len as u64)
}

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
///
/// The representation length is unbounded in `value` — a large enough
/// counter value drives it toward the platform's allocation limit, and (see
/// [`MAX_REPR_BYTES`]'s doc) so does an arbitrarily long `symbol` at a
/// small `value`. See [`MAX_REPR_BYTES`] for the spec-sanctioned cap
/// enforced here.
fn symbolic_repr(symbols: &[CounterSymbol], value: i64) -> Option<String> {
    let n = symbols.len() as i64;
    if n == 0 {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥1.
    }
    let index = (value - 1).rem_euclid(n);
    let symbol = symbols[index as usize].as_str();
    // ceil(value / n), computed as `(value - 1) / n + 1` rather than the
    // more obvious `(value + n - 1) / n` — the latter's intermediate
    // `value + n` can overflow `i64` for `value` near `i64::MAX`, before
    // the `- 1` would bring it back in range. `(value - 1)` cannot
    // underflow here (`value > 0` in this branch), and the final `+ 1`
    // cannot overflow either, since the result is `<= value <= i64::MAX`.
    let reps = if value <= 0 { 0 } else { (value - 1) / n + 1 };
    match checked_repr_bytes(reps, symbol.len()) {
        Some(bytes) if bytes <= MAX_REPR_BYTES => Some(symbol.repeat(reps as usize)),
        _ => None,
    }
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
/// Step 2's branch has a second `None` case beyond "no weight-0 tuple
/// exists": a matching tuple's `<symbol>` whose own byte length alone
/// exceeds [`MAX_REPR_BYTES`] (that constant's doc) also yields `None` here,
/// the same cap step 3's loop applies to every other tuple's contribution.
///
/// Step 3's per-tuple `reps` is unbounded in `value` — a large enough
/// counter value (paired with a small tuple weight) drives the cumulative
/// repetition count toward the platform's allocation limit, and (see
/// [`MAX_REPR_BYTES`]'s doc) so does an arbitrarily long tuple `symbol` at a
/// small `reps`. See [`MAX_REPR_BYTES`] for the spec-sanctioned cap enforced
/// here: the running byte total is checked *before* each tuple's
/// repetitions are pushed onto `s`, so the cap bounds `s`'s worst-case size
/// rather than merely detecting the overrun after the fact.
fn additive_repr(tuples: &[(i32, CounterSymbol)], value: i64) -> Option<String> {
    if tuples.is_empty() {
        return None; // cov:ignore: defensive; unreachable — `is_valid` requires ≥1.
    }
    if value == 0 {
        return tuples.iter().find(|(w, _)| *w == 0).and_then(|(_, sym)| {
            let symbol = sym.as_str();
            // Same byte-budget check as the loop below, applied here too —
            // a weight-0 tuple is parser-valid (`additive-symbols` accepts a
            // weight of 0, and additive's auto range includes 0, see
            // `auto_range`) and its `<symbol>` has no parser-enforced length
            // limit, so without this check an arbitrarily long symbol string
            // on a weight-0 tuple would allocate unboundedly even though
            // `reps` here is conceptually always 1.
            match checked_repr_bytes(1, symbol.len()) {
                Some(bytes) if bytes <= MAX_REPR_BYTES => Some(symbol.to_string()),
                _ => None,
            }
        });
    }
    let mut v = value;
    let mut s = String::new();
    let mut total_bytes: u64 = 0;
    for (weight, symbol) in tuples {
        let w = i64::from(*weight);
        if w == 0 || w > v {
            continue;
        }
        let reps = v / w;
        let symbol_str = symbol.as_str();
        let tuple_bytes = checked_repr_bytes(reps, symbol_str.len())?;
        // `total_bytes <= MAX_REPR_BYTES` is this loop's invariant (true
        // initially, and re-established below on every iteration that
        // doesn't already return), so `MAX_REPR_BYTES - total_bytes` never
        // underflows — comparing against the *remaining* budget this way,
        // rather than adding `tuple_bytes` (which can itself be near
        // `u64::MAX`, see `checked_repr_bytes`'s doc) to `total_bytes` and
        // checking afterward, means the running total itself never needs
        // its own overflow check.
        if tuple_bytes > MAX_REPR_BYTES - total_bytes {
            return None;
        }
        total_bytes += tuple_bytes;
        // The byte budget above cannot see this loop's *time* cost: an
        // empty `symbol_str` makes `tuple_bytes` (and therefore its
        // contribution to `total_bytes`) `0` regardless of `reps`, so a
        // zero-weight-adjacent tuple with an empty symbol and a `reps` in
        // the billions (bounded only by the counter value, not by this
        // function's byte budget) would otherwise still run this loop
        // `reps` times doing nothing — a CPU-time DoS the byte budget
        // doesn't close on its own. Skipping the loop entirely when
        // `symbol_str` is empty is a no-op-preserving optimization (`reps`
        // pushes of `""` never change `s`), not a behavior change.
        if !symbol_str.is_empty() {
            for _ in 0..reps {
                s.push_str(symbol_str);
            }
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
/// lives there, in `format_counter` / `format_named_counter`.
/// Reimplementing it here would duplicate that source of truth. The paint
/// consumer treats `None` from this function as the CSS decimal fallback,
/// while a successful result
/// is rendered as the custom style's representation. Prefix/suffix application
/// for a default `::marker` stays at that consumer boundary because these
/// descriptors do not apply to `counter()` / `counters()` themselves.
pub fn resolve_custom_counter(
    registry: &CounterStyleRegistry,
    name: &str,
    value: i32,
) -> Option<String> {
    let mut visited = HashSet::new();
    generate_counter(registry, name, value, &mut visited)
}

fn generate_counter<'a>(
    registry: &'a CounterStyleRegistry,
    name: &str,
    value: i32,
    visited: &mut HashSet<&'a str>,
) -> Option<String> {
    // Step 1.
    let mut rule = registry.get(name)?;
    // Per §3.7's explicit loop-detection rule ("if a loop in the specified
    // fallbacks is detected, the decimal style must be used instead" — full
    // quote + anchor on this function's doc), modeled here via the crate's
    // `None`-defers-to-`decimal` boundary convention: refuse to loop through
    // a `fallback` chain that revisits a name.
    //
    // Fallback resolution is deliberately iterative: CSS does not impose a
    // maximum fallback-chain length, so using the process stack for each hop
    // would make a long finite chain a stack-exhaustion vector.
    loop {
        // The keys borrow immutable registry entries for this resolution, so
        // no per-hop name clone is allocated. The set owns only O(N) hash
        // buckets and pointers, while expected O(1) lookup avoids the old
        // Vec's O(N) scan at every hop.
        if !visited.insert(rule.name.as_str()) {
            return None;
        }

        // Step 2.
        if !rule_contains_value(rule, value) {
            rule = registry.get(rule.fallback.as_str())?;
            continue;
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
        let Some(repr) = initial else {
            rule = registry.get(rule.fallback.as_str())?;
            continue;
        };

        // Step 4 — §3.6's pad algorithm (quoted in full on `apply_pad`) reduces
        // its padding `difference` by the negative descriptor's own length
        // *before* step 5 wraps it on, so the negative sign's presence still
        // counts toward `pad.min_length` even though it isn't part of `repr`
        // yet at this point in the algorithm. `apply_pad` itself can return
        // `None` (an author-supplied `pad.min_length` large enough, combined
        // with `pad.symbol`'s length *and* `repr`'s own byte length, to exceed
        // `MAX_REPR_BYTES` — see that constant's doc) — routed through the same
        // fallback hop as a `None` initial representation above, per this
        // function's own doc.
        let negative_reserved = if uses_negative && value_i64 < 0 {
            negative_descriptor_len(&rule.negative)
        } else {
            0
        };
        let Some(mut repr) = apply_pad(&rule.pad, repr, negative_reserved) else {
            rule = registry.get(rule.fallback.as_str())?;
            continue;
        };

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
        return Some(repr);
    }
}

#[cfg(test)]
mod tests;
