//! `@font-face` at-rule parsing + a family-name→rule registry.
//!
//! # Primary source
//!
//! CSS Fonts Level 4, §4 "The @font-face Rule"
//! <https://www.w3.org/TR/css-fonts-4/#font-face-rule>. Every spec-derived
//! branch below cites its own anchor; this header only states the top-level
//! section the rest of the module hangs off of.
//!
//! # Scope
//!
//! This module implements `@font-face` descriptor parsing and the registry.
//! Fetching a `src: url(...)` target and registering the bytes into a font
//! collection happen downstream (`raikiri-dom`'s `build_inline_document_fonts`
//! with a loader the caller provides) — this module has **no dependency on `raikiri-traits`, `raikiri-net`, or `raikiri-dom`**
//! and is not wired into any of those crates. See [`FontFaceRegistry`]'s doc
//! for exactly what a caller gets back and why.
//!
//! What's implemented:
//!
//! - Descriptor grammar: `font-family`, `src`, `font-style`, `font-weight`,
//!   `font-stretch` (plus `font-width`, accepted as an alias into the same
//!   field — CSS Fonts 4 §4.3 renamed `font-stretch` to `font-width` but
//!   keeps `font-stretch` as a legacy alias), `unicode-range`, and
//!   `font-display`. Anything else (e.g. `ascent-override`,
//!   `descent-override`, `line-gap-override`, `size-adjust`,
//!   `font-feature-settings`, `font-variation-settings`,
//!   `font-named-instance`) is deliberately **not** in that list and is
//!   silently dropped like any other unsupported descriptor or at-rule, per
//!   this crate's general convention (e.g. [`crate::ruletree`]'s
//!   `@media`/`@supports`, [`crate::counter_style`]'s `speak-as`, or an
//!   unrecognized nested at-rule inside `@page`).
//! - `src` component grammar: `url(...)` (bare token or function form) with
//!   optional trailing `format(...)` / `tech(...)` hints, and `local(...)`
//!   with a string or ident-sequence name. A component that is neither is
//!   dropped without invalidating the rule's other components; `format(...)`
//!   is recorded on its preceding `url(...)` source so a resource consumer can
//!   select the appropriate decoder, while `tech(...)` is
//!   consumed and dropped (its arguments are font-technology predicates with
//!   no bearing on the parse/registry use case this module serves).
//! - Whole-rule validity — CSS Fonts 4 §4's descriptor table marks
//!   `font-family` and `src` as required: a rule with a missing or empty
//!   `font-family`, or with no valid `src` source, defines no font and is
//!   dropped at the [`FontFaceRegistry::insert`] boundary (never stored),
//!   per this crate's general "spec-invalid → silently dropped" convention
//!   (`@page`'s invalid selector handling is the sibling precedent).
//!
//! What's out of scope, beyond the bullets above:
//!
//! - `src: url(...)` fetching and byte registration (see the "Scope" section
//!   above). A parsed [`FontFaceSource::Url`] carries the raw URL string and
//!   the optional `format(...)` hint so a future consumer
//!   (`raikiri-traits::ResourceKind::Font` policy + `raikiri-net` fetch +
//!   registration under the face's family name) has everything
//!   the stylesheet said; until that consumer exists, an unavailable `url`
//!   source simply never resolves — fail-closed, never a parse or cascade
//!   error.
//! - Weight/style/stretch/ranges/display *matching* (choosing which face
//!   serves a given element). The parsed descriptors are stored verbatim so
//!   the matcher has them; the matcher itself is a separate task.
//!
//! # RuleTree integration (origin-aware)
//!
//! [`crate::ruletree::RuleTree`] owns a `font_faces` [`FontFaceRegistry`],
//! populated while parsing stylesheet and conditional group rules, regardless
//! of `origin`. Read unconditional rules through
//! [`crate::ruletree::RuleTree::font_faces`] or include matching media rules
//! through [`crate::ruletree::RuleTree::font_faces_for`].
//!
//! Same-name resolution follows CSS Fonts 4 §4's "the last @font-face rule
//! ... wins" ordering through the standard cascade (origin first, then
//! source order within an origin), implemented with the same
//! [`crate::cascade::cascade_rank`]-based precedence
//! [`crate::counter_style::CounterStyleRegistry`] uses — see
//! [`FontFaceRegistry`]'s doc for the exact resolution table.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token, UnicodeRange,
};
use smol_str::SmolStr;

use crate::cascade::cascade_rank;
use crate::ruletree::{Origin, ParseBudget};

// ---------------------------------------------------------------------------
// Source — the `src` descriptor's `<font-src>` components.
// ---------------------------------------------------------------------------

/// Maximum number of `src` sources retained per `@font-face` rule.
///
/// An author-supplied `src` list is comma-separated with no spec-defined
/// upper bound, so an unbounded `Vec` here would let a single declaration
/// dictate allocation size. Past this cap, further valid sources are
/// silently dropped (the rule itself stays valid as long as ≥1 source was
/// retained — source order is preserved, so the author's preferred sources
/// come first and survive). The value comfortably exceeds real-world `src`
/// lists (a `local()` + a handful of `url()` fallbacks with `format()`
/// hints); it is a DoS bound, not a spec limit.
const MAX_FONT_FACE_SOURCES: usize = 64;

/// One `<url-source>` / `<local-source>` component of the `src` descriptor
/// (CSS Fonts 4 §4.2 <https://www.w3.org/TR/css-fonts-4/#font-face-src>).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontFaceSource {
    /// `url(<string>)` — bare `url(...)` token or `url()` function form.
    /// `format` carries the first `format(...)` hint following this source,
    /// if any (`format("woff2")`, `format("truetype")`, …). `tech(...)`
    /// hints are consumed and dropped (module doc "What's implemented").
    Url {
        url: SmolStr,
        format: Option<SmolStr>,
    },
    /// `local(<family-name>)` — system-installed font lookup by name. The
    /// name is a string or an ident sequence (`local(Times New Roman)`),
    /// stored verbatim; whether it resolves is the consumer's decision.
    Local(SmolStr),
}

// ---------------------------------------------------------------------------
// Descriptor value types.
// ---------------------------------------------------------------------------

/// `font-style` descriptor (CSS Fonts 4 §4.4
/// <https://www.w3.org/TR/css-fonts-4/#font-face-font-style>), narrowed to
/// the bare keywords. `oblique <angle>{0,2}` trailing angles are accepted
/// and dropped — they refine which oblique face matches, which is the
/// deferred matcher's concern, not the parser's.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontFaceStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

/// `font-weight` descriptor (CSS Fonts 4 §4.5
/// <https://www.w3.org/TR/css-fonts-4/#font-face-font-weight>).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FontFaceWeight {
    /// `normal` — same as `400`.
    #[default]
    Normal,
    /// `bold` — same as `700`.
    Bold,
    /// `<number>` in `1..=1000`.
    Number(f32),
    /// `<number> <number>` — both in `1..=1000`, first ≤ second.
    Range(f32, f32),
}

/// `font-stretch` descriptor (CSS Fonts 4 §4.6
/// <https://www.w3.org/TR/css-fonts-4/#font-face-font-stretch>), stored as
/// the raw keyword or percentage text. The descriptor grammar
/// (`normal | <percentage> | ultra-condensed … ultra-expanded`) is keyword-
/// heavy with no arithmetic the parser needs to do — the deferred matcher
/// interprets it. `font-width` is accepted as an alias into this same field
/// (§4.6's rename note).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontFaceStretch(pub SmolStr);

/// `font-display` descriptor (CSS Fonts 4 §4.8
/// <https://www.w3.org/TR/css-fonts-4/#font-face-font-display>).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontFaceDisplay {
    #[default]
    Auto,
    Block,
    Swap,
    Fallback,
    Optional,
}

// ---------------------------------------------------------------------------
// Rule.
// ---------------------------------------------------------------------------

/// A parsed `@font-face` rule — one field per implemented descriptor.
/// Registry insertion enforces that only spec-valid rules are retained.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct FontFaceRule {
    /// The rule's own `<family-name>` — registry key. Single name only
    /// (string or ident sequence); the descriptor takes no comma list.
    pub family: SmolStr,
    /// Valid `src` sources in author order (capped at
    /// [`MAX_FONT_FACE_SOURCES`]).
    pub src: Vec<FontFaceSource>,
    pub style: FontFaceStyle,
    pub weight: FontFaceWeight,
    pub stretch: FontFaceStretch,
    /// `unicode-range` ranges. Empty means "not specified", i.e. the spec
    /// default `U+0-10FFFF` (full range) — stored as empty rather than as an
    /// explicit full-range entry so "authored a range" vs "did not" stays
    /// distinguishable to the future matcher.
    pub unicode_range: Vec<(u32, u32)>,
    pub display: FontFaceDisplay,
}

impl FontFaceRule {
    /// A fresh rule for `family` with every descriptor at its spec-defined
    /// initial value (CSS Fonts 4 §4's descriptor table — `font-style:
    /// normal`, `font-weight: normal`, `font-stretch: normal`,
    /// `unicode-range: U+0-10FFFF` (stored as empty — see the field doc),
    /// `font-display: auto`; `src` has no initial value and starts empty,
    /// which is what makes a fresh rule invalid until `src` is set).
    pub fn new(family: SmolStr) -> Self {
        Self {
            family,
            src: Vec::new(),
            style: FontFaceStyle::Normal,
            weight: FontFaceWeight::Normal,
            stretch: FontFaceStretch(SmolStr::new("normal")),
            unicode_range: Vec::new(),
            display: FontFaceDisplay::Auto,
        }
    }

    /// Whole-rule validity — CSS Fonts 4 §4: `font-family` and `src` are
    /// required descriptors. An empty family (missing descriptor, or a
    /// descriptor that parsed to nothing) or zero valid sources means "the
    /// `@font-face` rule does not define a font" and the rule is dropped at
    /// the [`FontFaceRegistry::insert`] boundary.
    pub fn is_valid(&self) -> bool {
        !self.family.is_empty() && !self.src.is_empty()
    }
}

/// One successfully-parsed descriptor, tagged by which one it is — the
/// per-declaration output of [`FontFaceDeclParser`], folded into a
/// [`FontFaceRule`] by [`build_rule`] (later declarations of the same
/// descriptor win, matching ordinary CSS declaration-list semantics — the
/// same "later wins" fold [`crate::rule::parse_declaration_block_within`]'s callers
/// rely on for the cascade).
enum ParsedDescriptor {
    Family(SmolStr),
    Src(Vec<FontFaceSource>),
    Style(FontFaceStyle),
    Weight(FontFaceWeight),
    Stretch(FontFaceStretch),
    UnicodeRange(Vec<(u32, u32)>),
    Display(FontFaceDisplay),
}

fn parse_descriptor_value(name: &str, input: &mut Parser<'_, '_>) -> Option<ParsedDescriptor> {
    match name.to_ascii_lowercase().as_str() {
        "font-family" => parse_family_name(input).map(ParsedDescriptor::Family),
        "src" => parse_src(input).map(ParsedDescriptor::Src),
        "font-style" => parse_style(input).map(ParsedDescriptor::Style),
        "font-weight" => parse_weight(input).map(ParsedDescriptor::Weight),
        "font-stretch" | "font-width" => parse_stretch(input).map(ParsedDescriptor::Stretch),
        "unicode-range" => parse_unicode_range_list(input).map(ParsedDescriptor::UnicodeRange),
        "font-display" => parse_display(input).map(ParsedDescriptor::Display),
        // `ascent-override` / `size-adjust` / `font-feature-settings` and
        // anything unrecognized: silently dropped (module doc
        // "What's implemented").
        _ => None,
    }
}

/// Per-declaration parser for the `@font-face` block's `<declaration-list>`
/// — mirrors [`mod@crate::rule`]'s `DeclParser` shape exactly (same
/// `RuleBodyItemParser` integration, same "unsupported name / invalid value →
/// `Err` → whole declaration silently dropped by `RuleBodyParser`'s error
/// recovery" behavior), specialized to font-face descriptor names instead
/// of CSS property names.
struct FontFaceDeclParser;

impl<'i> DeclarationParser<'i> for FontFaceDeclParser {
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

// Nested at-rules / qualified rules inside a `@font-face` block are not
// part of the Fonts 4 grammar (`<declaration-list>` only) — no-op, matching
// `DeclParser`'s identical arms for the same reason.
impl<'i> AtRuleParser<'i> for FontFaceDeclParser {
    type Prelude = ();
    type AtRule = ParsedDescriptor;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for FontFaceDeclParser {
    type Prelude = ();
    type QualifiedRule = ParsedDescriptor;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, ParsedDescriptor, ()> for FontFaceDeclParser {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

fn parse_font_face_block(input: &mut Parser<'_, '_>) -> Vec<ParsedDescriptor> {
    let mut parser = FontFaceDeclParser;
    RuleBodyParser::new(input, &mut parser).flatten().collect()
}

/// Parse one descriptor block with the same recovery and validity as the standalone parser.
/// Crate-visible so stylesheet and conditional group parsing share this grammar.
///
/// Each descriptor counts as a declaration against `budget` as it is parsed,
/// so a block past the limit stops there and yields no rule.
pub(crate) fn parse_font_face_rule(
    input: &mut Parser<'_, '_>,
    budget: &mut ParseBudget,
) -> Option<FontFaceRule> {
    let mut parser = FontFaceDeclParser;
    let mut descriptors = Vec::new();
    for descriptor in RuleBodyParser::new(input, &mut parser).flatten() {
        if !budget.declarations(1) {
            return None;
        }
        descriptors.push(descriptor);
    }
    build_rule(None, descriptors)
}

fn build_rule(family: Option<SmolStr>, descriptors: Vec<ParsedDescriptor>) -> Option<FontFaceRule> {
    // `font-family` may arrive as a descriptor like any other (later-wins
    // fold), but the rule's prelude carries no name — unlike
    // `@counter-style`, `@font-face` has an empty prelude and names itself
    // via the descriptor. Start from an empty family; a rule with no valid
    // `font-family` descriptor fails `is_valid` below.
    let mut rule = FontFaceRule::new(family.unwrap_or_else(|| SmolStr::new("")));
    for d in descriptors {
        match d {
            ParsedDescriptor::Family(v) => rule.family = v,
            ParsedDescriptor::Src(v) => rule.src = v,
            ParsedDescriptor::Style(v) => rule.style = v,
            ParsedDescriptor::Weight(v) => rule.weight = v,
            ParsedDescriptor::Stretch(v) => rule.stretch = v,
            ParsedDescriptor::UnicodeRange(v) => rule.unicode_range = v,
            ParsedDescriptor::Display(v) => rule.display = v,
        }
    }
    if rule.is_valid() { Some(rule) } else { None }
}

// ---------------------------------------------------------------------------
// Descriptor parsers.
// ---------------------------------------------------------------------------

/// `font-family: <family-name>` — a single name: a string, or a sequence of
/// idents joined with single spaces (`Times New Roman`). Empty strings and
/// empty ident runs are rejected (they would make the rule invalid anyway
/// via [`FontFaceRule::is_valid`]; rejecting here keeps the declaration
/// dropped rather than folding an empty name over an earlier valid one).
fn parse_family_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    if let Ok(s) = input.try_parse(|input| input.expect_string_cloned()) {
        if s.is_empty() {
            return None;
        }
        return Some(SmolStr::new(s.as_ref()));
    }
    let first = input.try_parse(|input| input.expect_ident_cloned()).ok()?;
    let mut name = first.to_string();
    while let Ok(next) = input.try_parse(|input| input.expect_ident_cloned()) {
        name.push(' ');
        name.push_str(next.as_ref());
    }
    Some(SmolStr::new(name))
}

/// `src: [<url> [format(...)|tech(...)]? | local(...)]#` — comma-separated
/// components; invalid components are dropped individually (via
/// `parse_comma_separated_ignoring_errors`), valid ones kept in author
/// order up to [`MAX_FONT_FACE_SOURCES`]. Returns `None` when no valid
/// source remains (the declaration is dropped; the rule then fails
/// [`FontFaceRule::is_valid`] unless an earlier valid `src` was folded).
fn parse_src(input: &mut Parser<'_, '_>) -> Option<Vec<FontFaceSource>> {
    let mut sources: Vec<FontFaceSource> =
        input.parse_comma_separated_ignoring_errors(|input| parse_one_src(input));
    if sources.is_empty() {
        return None;
    }
    sources.truncate(MAX_FONT_FACE_SOURCES);
    Some(sources)
}

/// One comma-separated `src` component: a leading `url(...)` / `local(...)`
/// followed by zero or more `format(...)` / `tech(...)` hints. A component
/// with no leading source (hint-only, or anything else) is an error and is
/// dropped by the caller's ignoring-errors loop — without touching sibling
/// components.
fn parse_one_src<'i>(input: &mut Parser<'i, '_>) -> Result<FontFaceSource, ParseError<'i, ()>> {
    let mut source = parse_src_head(input)?;
    loop {
        if let Ok(format) = input.try_parse(|input| parse_format_hint(input)) {
            if let FontFaceSource::Url { format: slot, .. } = &mut source
                && slot.is_none()
            {
                *slot = Some(format);
            }
            continue;
        }
        if input
            .try_parse(|input| consume_simple_function(input, "tech"))
            .is_ok()
        {
            continue;
        }
        break;
    }
    input.expect_exhausted()?;
    Ok(source)
}

/// The leading source of one `src` component: `url(...)` or `local(...)`.
fn parse_src_head<'i>(input: &mut Parser<'i, '_>) -> Result<FontFaceSource, ParseError<'i, ()>> {
    // Bare `url(...)` token first — `expect_url` also accepts the `url()`
    // function form, so this single call covers both spellings.
    if let Ok(url) = input.try_parse(|input| input.expect_url()) {
        return Ok(FontFaceSource::Url {
            url: SmolStr::new(url.as_ref()),
            format: None,
        });
    }
    // `local(<family-name>)`.
    if input
        .try_parse(|input| input.expect_function_matching("local"))
        .is_ok()
    {
        let name = input.parse_nested_block(|input| parse_local_name(input))?;
        return Ok(FontFaceSource::Local(name));
    }
    Err(input.new_custom_error(()))
}

/// `local()` argument: a string or an ident sequence, same shape as
/// [`parse_family_name`] but without the empty check duplicated — an empty
/// `local("")` is dropped here instead (it names no installed font, so
/// keeping it would only add a never-resolving source).
fn parse_local_name<'i>(input: &mut Parser<'i, '_>) -> Result<SmolStr, ParseError<'i, ()>> {
    if let Ok(s) = input.try_parse(|input| input.expect_string_cloned()) {
        if s.is_empty() {
            return Err(input.new_custom_error(()));
        }
        return Ok(SmolStr::new(s.as_ref()));
    }
    let first = input.expect_ident_cloned()?;
    let mut name = first.to_string();
    while let Ok(next) = input.try_parse(|input| input.expect_ident_cloned()) {
        name.push(' ');
        name.push_str(next.as_ref());
    }
    input.expect_exhausted()?;
    Ok(SmolStr::new(name))
}

/// `format(<string> | <ident>)` — consumes the whole function, returns the
/// first string/ident argument. Remaining arguments (the `format()` grammar
/// allows only one, but error recovery must not choke on more) are drained.
fn parse_format_hint<'i>(input: &mut Parser<'i, '_>) -> Result<SmolStr, ParseError<'i, ()>> {
    input.expect_function_matching("format")?;
    input.parse_nested_block(|input| {
        let first: SmolStr = input
            .try_parse(|input| {
                input
                    .expect_string_cloned()
                    .map(|s| SmolStr::new(s.as_ref()))
                    .or_else(|_| {
                        input
                            .expect_ident_cloned()
                            .map(|s| SmolStr::new(s.as_ref()))
                    })
            })
            .map_err(|_: cssparser::BasicParseError<'_>| input.new_custom_error(()))?;
        // Drain anything else in the function so a malformed `format(a b)`
        // still consumes to the closing paren.
        while input.next().is_ok() {}
        Ok(first)
    })
}

/// Consume a whole `name(...)` function, discarding its arguments (used for
/// `tech(...)`, whose font-technology predicates carry no parse-time
/// information). Returns `Err` when the next token is not that function, so
/// callers can `try_parse` it in a hint loop.
fn consume_simple_function<'i>(
    input: &mut Parser<'i, '_>,
    name: &str,
) -> Result<(), ParseError<'i, ()>> {
    input.expect_function_matching(name)?;
    input.parse_nested_block(|input| {
        while input.next().is_ok() {}
        Ok(())
    })
}

/// `font-style: normal | italic | oblique [<angle>{0,2}]?`.
fn parse_style(input: &mut Parser<'_, '_>) -> Option<FontFaceStyle> {
    let ident = input.expect_ident_cloned().ok()?;
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "normal" => Some(FontFaceStyle::Normal),
        "italic" => Some(FontFaceStyle::Italic),
        "oblique" => {
            // Optional trailing angles refine oblique matching (deferred to
            // the matcher) — consume and drop up to two `<angle>` tokens so
            // `oblique 10deg` still parses instead of failing exhaustive
            // consumption at the declaration level.
            for _ in 0..2 {
                let is_angle = matches!(input.next(), Ok(Token::Dimension { unit, .. }) if {
                    let u: &str = unit.as_ref();
                    u.eq_ignore_ascii_case("deg")
                        || u.eq_ignore_ascii_case("grad")
                        || u.eq_ignore_ascii_case("rad")
                        || u.eq_ignore_ascii_case("turn")
                });
                if !is_angle {
                    break;
                }
            }
            Some(FontFaceStyle::Oblique)
        }
        _ => None,
    }
}

/// `font-weight: normal | bold | <number> | <number> <number>`.
fn parse_weight(input: &mut Parser<'_, '_>) -> Option<FontFaceWeight> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        return match ident.as_ref().to_ascii_lowercase().as_str() {
            "normal" => Some(FontFaceWeight::Normal),
            "bold" => Some(FontFaceWeight::Bold),
            _ => None,
        };
    }
    let first = input.expect_number().ok()?;
    if !(1.0..=1000.0).contains(&first) {
        return None;
    }
    if let Ok(second) = input.try_parse(|input| input.expect_number()) {
        if !(1.0..=1000.0).contains(&second) || second < first {
            return None;
        }
        return Some(FontFaceWeight::Range(first, second));
    }
    Some(FontFaceWeight::Number(first))
}

/// `font-stretch: normal | <percentage> | ultra-condensed … ultra-expanded`
/// (and the `font-width` alias spelling — same value space). Stored verbatim
/// (keyword lowercased, percentage as authored); interpretation is the
/// deferred matcher's job.
fn parse_stretch(input: &mut Parser<'_, '_>) -> Option<FontFaceStretch> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        let lower = ident.as_ref().to_ascii_lowercase();
        let known = matches!(
            lower.as_str(),
            "normal"
                | "ultra-condensed"
                | "extra-condensed"
                | "condensed"
                | "semi-condensed"
                | "semi-expanded"
                | "expanded"
                | "extra-expanded"
                | "ultra-expanded"
        );
        if !known {
            return None;
        }
        return Some(FontFaceStretch(SmolStr::new(lower)));
    }
    if let Ok(percentage) = input.try_parse(|input| input.expect_percentage()) {
        return Some(FontFaceStretch(SmolStr::new(format!("{percentage}%"))));
    }
    None
}

/// `unicode-range: <unicode-range>#` via cssparser's own `U+…` tokenizer —
/// no hand transcription of the hex grammar. An empty result (all
/// components invalid) drops the declaration.
fn parse_unicode_range_list(input: &mut Parser<'_, '_>) -> Option<Vec<(u32, u32)>> {
    let ranges: Vec<(u32, u32)> = input.parse_comma_separated_ignoring_errors(|input| {
        UnicodeRange::parse(input)
            .map(|r| (r.start, r.end))
            .map_err(|_| input.new_custom_error::<(), ()>(()))
    });
    if ranges.is_empty() {
        return None;
    }
    Some(ranges)
}

/// `font-display: auto | block | swap | fallback | optional`.
fn parse_display(input: &mut Parser<'_, '_>) -> Option<FontFaceDisplay> {
    let ident = input.expect_ident_cloned().ok()?;
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "auto" => Some(FontFaceDisplay::Auto),
        "block" => Some(FontFaceDisplay::Block),
        "swap" => Some(FontFaceDisplay::Swap),
        "fallback" => Some(FontFaceDisplay::Fallback),
        "optional" => Some(FontFaceDisplay::Optional),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Stylesheet-level extraction — `@font-face` rules only.
// ---------------------------------------------------------------------------

/// Top-level parsed item this module's stylesheet scan can produce.
/// Mirrors [`crate::counter_style`]'s `TopLevelItem` shape for the same
/// cssparser reason (`AtRuleParser::AtRule` and
/// `QualifiedRuleParser::QualifiedRule` must be the same type).
enum TopLevelItem {
    FontFace(Vec<ParsedDescriptor>),
}

/// Standalone stylesheet parser that accepts only `@font-face` blocks.
/// [`crate::ruletree::RuleTree`] collects individual rules while parsing
/// stylesheets and conditional groups.
struct FontFaceSheetParser;

impl<'i> AtRuleParser<'i> for FontFaceSheetParser {
    type Prelude = ();
    type AtRule = TopLevelItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("font-face") {
            // `@font-face` takes no prelude — anything before the block
            // invalidates the rule.
            input.expect_exhausted()?;
            Ok(())
        } else {
            // @media / @page / @counter-style / etc — not this module's
            // concern; cssparser's error recovery skips the whole block.
            Err(input.new_custom_error(()))
        }
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        // Statement-form `@font-face;` never reaches here (cssparser routes
        // prelude-without-block to `rule_without_block`, which this impl
        // does not provide, so the rule is dropped) — only real blocks do.
        let descriptors = parse_font_face_block(input);
        Ok(TopLevelItem::FontFace(descriptors))
    }
}

impl<'i> QualifiedRuleParser<'i> for FontFaceSheetParser {
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

/// Parse every `@font-face` at-rule out of `source` (a full stylesheet
/// text — e.g. the concatenated text of a `<style>` element, same unit
/// [`crate::ruletree::RuleTree::add_stylesheet`] takes). Everything else in
/// `source` (style rules, other at-rules) is silently ignored. A rule that
/// fails [`FontFaceRule::is_valid`] is **not** included — matches
/// `RuleTree`'s "invalid rule → dropped" convention for `@page`.
pub fn parse_font_face_rules(source: &str) -> Vec<FontFaceRule> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut sheet_parser = FontFaceSheetParser;
    let mut out = Vec::new();
    for item in StyleSheetParser::new(&mut parser, &mut sheet_parser).flatten() {
        let TopLevelItem::FontFace(descriptors) = item;
        if let Some(rule) = build_rule(None, descriptors) {
            out.push(rule);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// FontFaceRegistry
// ---------------------------------------------------------------------------

/// Family-name → [`FontFaceRule`] registry.
///
/// Same-name rules follow the standard cascade (origin first, then source
/// order within an origin) using [`crate::cascade::cascade_rank`]. The
/// registry stores complete rules and does not fetch sources or select faces.
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct FontFaceRegistry {
    rules: HashMap<SmolStr, (Origin, FontFaceRule)>,
}

impl FontFaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` and insert every valid `@font-face` rule it contains
    /// (later same-name rules replace earlier ones — see [`Self::insert`]'s
    /// doc).
    pub fn from_source(source: &str) -> Self {
        let mut registry = Self::new();
        for rule in parse_font_face_rules(source) {
            registry.insert(rule);
        }
        registry
    }

    /// Insert `rule`, replacing any existing rule of the same family
    /// entirely, **regardless of that existing entry's origin** — this
    /// method does not participate in the [`Origin`]-aware precedence
    /// [`Self::insert_with_origin`] implements (type doc). A rule that fails
    /// [`FontFaceRule::is_valid`] is silently dropped (does not replace an
    /// existing valid rule of the same family) — this is the one enforcement
    /// point for "only valid rules live in a registry", covering both the
    /// [`parse_font_face_rules`] entry path and direct construction of a
    /// [`FontFaceRule`] via its `pub` fields.
    pub fn insert(&mut self, rule: FontFaceRule) {
        if rule.is_valid() {
            self.rules
                .insert(rule.family.clone(), (Origin::Author, rule));
        }
    }

    /// Insert `rule` with an origin-aware same-name precedence. Invalid rules
    /// are silently dropped, as in [`Self::insert`].
    pub(crate) fn insert_with_origin(&mut self, rule: FontFaceRule, origin: Origin) {
        if !rule.is_valid() {
            return;
        }
        if let Some((existing_origin, _)) = self.rules.get(&rule.family)
            && cascade_rank(origin, false) < cascade_rank(*existing_origin, false)
        {
            return; // Lower-ranked origin never overwrites a higher-ranked one.
        }
        self.rules.insert(rule.family.clone(), (origin, rule));
    }

    pub fn get(&self, family: &str) -> Option<&FontFaceRule> {
        self.rules.get(family).map(|(_, rule)| rule)
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Iterate over every stored rule (family-name order is unspecified —
    /// `HashMap` iteration). Used by consumers that need the whole
    /// registry (e.g. a future font-context seeding pass).
    pub fn iter(&self) -> impl Iterator<Item = (&SmolStr, &FontFaceRule)> {
        self.rules.values().map(|(_, rule)| (&rule.family, rule))
    }
}

#[cfg(test)]
mod tests;
