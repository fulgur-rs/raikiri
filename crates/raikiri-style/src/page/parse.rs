use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, Token, match_ignore_ascii_case,
};

use crate::Atom;
use crate::property::{
    Length, PropertyValue, parse_length_allow_negative, parse_non_negative_length, parse_value,
};
use crate::rule::{Declaration, expand_shorthand_into, parse_declaration_block};

use super::types::*;

/// Parse the prelude of an `@page` rule.
///
/// The caller (a cssparser `AtRuleParser::parse_prelude`) hands us a parser
/// that ends at the block's opening `{`. We accept the full L3
/// `<page-selector-list>` grammar (see module docs). An empty prelude
/// (`@page { … }`) becomes a [`PageSelector`] with one default-constructed
/// entry so downstream cascade code can treat "matches every page" as an
/// entry that matches unconditionally.
///
/// Anything violating the compound rule — whitespace between an ident and
/// its pseudo-page, whitespace between a `:` and its pseudo keyword, or
/// whitespace between two pseudo-pages of the same compound — returns
/// `Err`, which cssparser converts into "drop the whole @page rule".
pub(crate) fn parse_page_prelude<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSelector, ParseError<'i, ()>> {
    // Empty prelude → `@page { … }` matches every page. Represented as a
    // single empty entry so the cascade code can iterate uniformly.
    if input.is_exhausted() {
        return Ok(PageSelector {
            entries: vec![PageSelectorEntry::default()],
        });
    }

    // `parse_comma_separated` skips whitespace around each `,` (which is
    // spec-permitted since the list is not a compound) and, per its
    // `parse_until_before` / `parse_entirely` internals, enforces that each
    // closure invocation consumes every token up to the delimiter — so any
    // trailing garbage inside a compound propagates as an Err and the whole
    // `@page` rule is dropped.
    let entries = input.parse_comma_separated(parse_compound_selector)?;
    Ok(PageSelector { entries })
}

/// Parse a single compound `<page-selector>` = `[ <ident-token>?
/// <pseudo-page>* ]!`.
///
/// The compound rule (spec: "No whitespace is allowed between the
/// productions in `<page-selector>` or `<pseudo-page>`") is enforced by
/// using `Parser::next_including_whitespace` at every intra-compound
/// boundary — that primitive surfaces `Token::WhiteSpace(_)` but is
/// transparent to comments (comments are stripped at CSS Syntax L3
/// tokenization, so `:first/*x*/:left` must parse the same as
/// `:first:left`). Only the *first* component is read with
/// `Parser::next` (whitespace-skipping) because that skip covers the
/// spec-permitted whitespace between commas of the outer list and before
/// the very first entry.
pub(crate) fn parse_compound_selector<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSelectorEntry, ParseError<'i, ()>> {
    let mut entry = PageSelectorEntry::default();

    // First component: whitespace-skipping OK — we're either at the start of
    // the prelude or just past a `,`, both non-compound boundaries.
    let first = input.next()?.clone();
    match first {
        Token::Ident(name) => {
            entry.ident = Some(Atom::from(name.as_ref()));
        }
        Token::Colon => {
            parse_and_push_pseudo(input, &mut entry.pseudos)?;
        }
        other => {
            return Err(input.new_unexpected_token_error(other));
        }
    }

    // Additional pseudo-pages: each must be adjacent to the previous
    // component. Save the state before each peek so a non-`:` (or an EOI /
    // whitespace token) can rewind and let the outer `parse_comma_separated`
    // see what actually comes next (whitespace before a `,`, or EOI).
    //
    // `next_including_whitespace` is used (not `..._and_comments`) because
    // CSS Syntax L3 §4 strips comments at tokenization: `:first/*x*/:left`
    // MUST parse identically to `:first:left`. Whitespace, however, remains
    // a compound boundary — so this primitive surfaces `Token::WhiteSpace(_)`
    // and lets the "no whitespace within compound" rule stay strict.
    loop {
        let checkpoint = input.state();
        let peek = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => break, // EOI — compound is complete
        };
        match peek {
            Token::Colon => {
                parse_and_push_pseudo(input, &mut entry.pseudos)?;
            }
            _ => {
                // Not a `:` (could be whitespace, comma, or anything else).
                // Rewind — outer machinery handles it. Whitespace here is
                // fine because it's *after* the compound, not within it.
                input.reset(&checkpoint);
                break;
            }
        }
    }

    Ok(entry)
}

/// Consume the ident that must immediately follow a `:` in a `<pseudo-page>`
/// and push the matching [`PagePseudo`] variant.
///
/// `Parser::next_including_whitespace` is used so that a `@page : left { … }`
/// shape (whitespace between colon and ident) surfaces as
/// `Token::WhiteSpace(_)` rather than being silently skipped past — matching
/// the compound rule from L3. Comments between `:` and the ident
/// (`@page :/*x*/left`) are transparent per CSS Syntax L3 §4 (stripped at
/// tokenization); this primitive strips them but preserves whitespace.
pub(crate) fn parse_and_push_pseudo<'i>(
    input: &mut Parser<'i, '_>,
    pseudos: &mut Vec<PagePseudo>,
) -> Result<(), ParseError<'i, ()>> {
    let tok = input.next_including_whitespace()?.clone();
    match tok {
        Token::Ident(name) => {
            let pseudo = match_ignore_ascii_case! { &name,
                "first" => PagePseudo::First,
                "left"  => PagePseudo::Left,
                "right" => PagePseudo::Right,
                "blank" => PagePseudo::Blank,
                _ => return Err(input.new_custom_error(())),
            };
            pseudos.push(pseudo);
            Ok(())
        }
        // WhiteSpace here (`@page : left`), functional pseudo (`:nth-page(…)`),
        // or any other token type is invalid per the compound rule.
        _ => Err(input.new_custom_error(())),
    }
}

// ---------------------------------------------------------------------------
// @page block body — declarations + the `size` / `marks` / `bleed`
// descriptors
//
// CSS Paged Media Level 3:
//   §7.1 "Page size: the size property" —
//     <https://www.w3.org/TR/css-page-3/#page-size-prop>
//   §7.2 "Crop and Registration Marks: the marks property" —
//     <https://www.w3.org/TR/css-page-3/#marks>
//   §7.3 "Bleed Area: the bleed property" —
//     <https://www.w3.org/TR/css-page-3/#bleed>
// ---------------------------------------------------------------------------

/// Per-declaration parse result for an `@page` block body — either an
/// ordinary property [`Declaration`] (routed through the same
/// [`crate::property::parse_value`] dispatch qualified rules use), one of
/// the three descriptors' dedicated values ([`PageSize`] / [`PageMarks`] /
/// [`PageBleed`]), none of which are [`PropertyValue`] variants and so
/// cannot come out of that dispatch, or a nested margin-box at-rule
/// ([`PageMarginBoxRule`]), which is not a declaration at all.
pub(crate) enum PageBodyItem {
    /// An ordinary property declaration.
    Property(Declaration),
    /// A `size:` declaration.
    Size(PageSizeDeclaration),
    /// A `marks:` declaration.
    Marks(PageMarksDeclaration),
    /// A `bleed:` declaration.
    Bleed(PageBleedDeclaration),
    /// A margin-box at-rule (`@top-left { … }` etc.).
    MarginBox(PageMarginBoxRule),
}

/// Per-declaration parser for `@page` block bodies, driving
/// [`RuleBodyParser`] the same way [`crate::rule`]'s (crate-private)
/// `DeclParser` drives it for qualified style rules — see
/// [`parse_page_declaration_block`] for the entry point and the
/// `size`/`marks`/`bleed`/ordinary-property dispatch this type implements.
///
/// # `AtRuleParser` recognizes the sixteen margin-box at-rules
///
/// `QualifiedRuleParser` below is a no-op (all default-trait-method
/// behavior, same shape as [`crate::rule`]'s `DeclParser`): a nested
/// qualified/selector rule is not part of `@page` block grammar, so
/// cssparser's error recovery silently skips it (declarations before and
/// after it in the `@page` block still parse).
///
/// `AtRuleParser`, unlike `QualifiedRuleParser`, is *not* a no-op: it
/// recognizes a margin-box at-rule (`@top-left { … }` etc., CSS Paged Media
/// Level 3 §5.1 "At-rules for page-margin boxes",
/// <https://www.w3.org/TR/css-page-3/#margin-at-rules>) by ident and parses its
/// body as an ordinary declaration list — see the impl below, and
/// [`PageMarginBoxRule`]'s doc for the stored shape and its "Scope" section for
/// what is deliberately not done yet. Any other nested at-rule name still
/// falls through to `AtRuleParser::parse_prelude`'s rejection and
/// cssparser's error-recovery skip — declarations before and after it still
/// parse (pinned by
/// `ruletree::tests::page_unknown_nested_at_rule_body_is_skipped_declaration_survives`).
pub(crate) struct PageDeclParser;

impl<'i> DeclarationParser<'i> for PageDeclParser {
    type Declaration = PageBodyItem;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<PageBodyItem, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("size") {
            let value = parse_page_size_value(input)?;
            let important = parse_important_and_exhaust(input)?;
            return Ok(PageBodyItem::Size(PageSizeDeclaration { value, important }));
        }
        if name.eq_ignore_ascii_case("marks") {
            let value = parse_page_marks_value(input)?;
            let important = parse_important_and_exhaust(input)?;
            return Ok(PageBodyItem::Marks(PageMarksDeclaration {
                value,
                important,
            }));
        }
        if name.eq_ignore_ascii_case("bleed") {
            let value = parse_page_bleed_value(input)?;
            let important = parse_important_and_exhaust(input)?;
            return Ok(PageBodyItem::Bleed(PageBleedDeclaration {
                value,
                important,
            }));
        }
        let inherited_marker = match name.as_ref().to_ascii_lowercase().as_str() {
            "margin" => input
                .try_parse(|input| input.expect_ident_matching("inherit"))
                .is_ok()
                .then_some(PropertyValue::MarginInherit),
            "margin-top" => input
                .try_parse(|input| input.expect_ident_matching("inherit"))
                .is_ok()
                .then_some(PropertyValue::MarginTopInherit),
            "margin-right" => input
                .try_parse(|input| input.expect_ident_matching("inherit"))
                .is_ok()
                .then_some(PropertyValue::MarginRightInherit),
            "margin-bottom" => input
                .try_parse(|input| input.expect_ident_matching("inherit"))
                .is_ok()
                .then_some(PropertyValue::MarginBottomInherit),
            "margin-left" => input
                .try_parse(|input| input.expect_ident_matching("inherit"))
                .is_ok()
                .then_some(PropertyValue::MarginLeftInherit),
            _ => None,
        };
        let value = inherited_marker
            .or_else(|| parse_value(name.as_ref(), input))
            .ok_or_else(|| input.new_custom_error(()))?;
        let important = parse_important_and_exhaust(input)?;
        Ok(PageBodyItem::Property(Declaration { value, important }))
    }
}

// Recognizes the sixteen margin-box at-rule idents — see `PageDeclParser`'s
// doc for the split with `QualifiedRuleParser` below.
impl<'i> AtRuleParser<'i> for PageDeclParser {
    type Prelude = PageMarginBoxSlot;
    type AtRule = PageBodyItem;
    type Error = ();

    /// Match the at-rule name against the sixteen margin-box idents (CSS
    /// Paged Media Level 3 §5.1,
    /// <https://www.w3.org/TR/css-page-3/#margin-at-rules>). Any other name is
    /// rejected — cssparser's at-rule error recovery then skips just that
    /// nested block (see `PageDeclParser`'s doc). Margin-box at-rules take
    /// no prelude (the grammar is `@top-left { <declaration-list> }`, with
    /// nothing between the ident and `{`), so a non-empty prelude — e.g.
    /// `@top-left foo { … }` — is rejected the same way via
    /// `expect_exhausted`.
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let slot = match_ignore_ascii_case! { &name,
            "top-left-corner" => PageMarginBoxSlot::TopLeftCorner,
            "top-left" => PageMarginBoxSlot::TopLeft,
            "top-center" => PageMarginBoxSlot::TopCenter,
            "top-right" => PageMarginBoxSlot::TopRight,
            "top-right-corner" => PageMarginBoxSlot::TopRightCorner,
            "right-top" => PageMarginBoxSlot::RightTop,
            "right-middle" => PageMarginBoxSlot::RightMiddle,
            "right-bottom" => PageMarginBoxSlot::RightBottom,
            "bottom-right-corner" => PageMarginBoxSlot::BottomRightCorner,
            "bottom-right" => PageMarginBoxSlot::BottomRight,
            "bottom-center" => PageMarginBoxSlot::BottomCenter,
            "bottom-left" => PageMarginBoxSlot::BottomLeft,
            "bottom-left-corner" => PageMarginBoxSlot::BottomLeftCorner,
            "left-bottom" => PageMarginBoxSlot::LeftBottom,
            "left-middle" => PageMarginBoxSlot::LeftMiddle,
            "left-top" => PageMarginBoxSlot::LeftTop,
            _ => return Err(input.new_custom_error(())),
        };
        input.expect_exhausted()?;
        Ok(slot)
    }

    /// Parse the margin-box at-rule's body as an ordinary declaration list.
    /// The margin-box grammar has no descriptors of its own (unlike the
    /// `@page` block body itself, which additionally recognizes `size` /
    /// `marks` / `bleed` — see [`PageDeclParser::parse_value`]), so this
    /// reuses [`parse_declaration_block`] directly rather than routing
    /// through this type's `DeclarationParser` impl. See
    /// [`PageMarginBoxRule::declarations`] for the shorthand-expansion guarantee
    /// this inherits.
    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        let declarations = parse_declaration_block(input);
        Ok(PageBodyItem::MarginBox(PageMarginBoxRule {
            slot: prelude,
            declarations,
        }))
    }
}

// Qualified-rule parser (nested rule) is also a no-op — see `PageDeclParser`'s
// doc.
impl<'i> QualifiedRuleParser<'i> for PageDeclParser {
    type Prelude = ();
    type QualifiedRule = PageBodyItem;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, PageBodyItem, ()> for PageDeclParser {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

/// Consume a declaration's optional trailing `!important` and confirm no
/// further tokens remain — the tail every [`PageDeclParser::parse_value`] arm
/// shares (the three descriptor arms and the ordinary-property arm alike).
///
/// `!important` is spec-legal on every `@page`-context declaration — CSS
/// Paged Media Level 3's "Cascading in the page context" states page-context
/// declarations "cascade just like declarations in style rule for elements"
/// (cited in full on `PageRule::origin`) — so this consumes it (rather than
/// leaving it as trailing garbage for `expect_exhausted` below to reject) and
/// returns whether it was present, for the caller to retain on its own
/// `*Declaration`.
///
/// Exhaustive consumption matches [`mod@crate::rule`] の `DeclParser`: trailing
/// garbage after the value (and optional `!important`) rejects the whole
/// declaration.
pub(crate) fn parse_important_and_exhaust<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<bool, ParseError<'i, ()>> {
    let important = input.try_parse(cssparser::parse_important).is_ok();
    input.expect_exhausted()?;
    Ok(important)
}

/// [`parse_page_declaration_block`]'s return shape, and (see
/// [`crate::ruletree`]'s `ParsedRule::Page`) the sole payload a parsed
/// `@page` block carries alongside its selector — the five lists (four of
/// declarations, one of nested margin-box at-rules) a parsed `@page` block
/// body produces, field-named the same as [`PageRule`]'s matching fields.
///
/// A named struct keeps the five independently typed lists distinct while the
/// parsed block moves through the stylesheet parser.
pub(crate) struct PageBlockBody {
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) size_declarations: Vec<PageSizeDeclaration>,
    pub(crate) marks_declarations: Vec<PageMarksDeclaration>,
    pub(crate) bleed_declarations: Vec<PageBleedDeclaration>,
    pub(crate) margin_box_rules: Vec<PageMarginBoxRule>,
}

/// Parse an `@page` block body (the `{ … }` contents), producing the
/// ordinary property declarations, the `size:` / `marks:` / `bleed:`
/// declarations, and the nested margin-box at-rules (each list in source
/// order — see [`PageRule::size_declarations`] for why duplicates within one
/// block are kept rather than reduced here; [`PageMarginBoxRule`]'s doc gives
/// the same rationale for duplicate margin-box slots).
///
/// # Not a *direct* reuse of [`crate::rule::parse_declaration_block`] — except for margin boxes
///
/// Unlike qualified style rules, the `@page` block body itself is parsed by
/// a dedicated [`PageDeclParser`] rather than [`crate::rule`]'s
/// (crate-private) `DeclParser` — none of the three descriptors is a
/// [`PropertyValue`] variant [`crate::property::parse_value`] can produce,
/// so each needs its own grammar ([`parse_page_size_value`] /
/// [`parse_page_marks_value`] / [`parse_page_bleed_value`]) rather than
/// being silently dropped by the general property parser (the pre-existing
/// behavior for *every* `@page` descriptor before this function existed).
/// The margin-box at-rules are a different case again: they're not a
/// declaration at all, so `PageDeclParser::parse_value` never sees them —
/// `PageDeclParser`'s `AtRuleParser` impl handles them instead (see that
/// impl's doc), and *its* `parse_block` **does** reuse
/// [`crate::rule::parse_declaration_block`] directly, because a margin-box
/// at-rule's body has no descriptors of its own — it is exactly the
/// ordinary-declaration-list grammar that function already implements.
///
/// # Shorthand expansion
///
/// Ordinary declarations are expanded before they are stored in the page
/// declaration list. Margin-box bodies use the ordinary declaration parser.
pub(crate) fn parse_page_declaration_block(input: &mut Parser<'_, '_>) -> PageBlockBody {
    let mut parser = PageDeclParser;
    let mut declarations = Vec::new();
    let mut size_declarations = Vec::new();
    let mut marks_declarations = Vec::new();
    let mut bleed_declarations = Vec::new();
    let mut margin_box_rules = Vec::new();
    for item in RuleBodyParser::new(input, &mut parser).flatten() {
        match item {
            PageBodyItem::Property(decl) => {
                expand_shorthand_into(&decl, |d| declarations.push(d));
            }
            PageBodyItem::Size(decl) => size_declarations.push(decl),
            PageBodyItem::Marks(decl) => marks_declarations.push(decl),
            PageBodyItem::Bleed(decl) => bleed_declarations.push(decl),
            PageBodyItem::MarginBox(rule) => margin_box_rules.push(rule),
        }
    }
    PageBlockBody {
        declarations,
        size_declarations,
        marks_declarations,
        bleed_declarations,
        margin_box_rules,
    }
}

/// Parse the `size` descriptor's value grammar — CSS Paged Media Level 3
/// §7.1 "Page size: the size property"
/// (<https://www.w3.org/TR/css-page-3/#page-size-prop>), grammar (spec
/// verbatim): `<length>{1,2} | auto | [ <page-size> || [ portrait |
/// landscape ] ]`. See [`PageSize`] for the parsed shape.
///
/// Called with the parser positioned right after `size` `:` (the same shape
/// [`crate::property::parse_value`] callers expect) — consumes exactly the
/// value tokens, leaving any trailing `!important` for the caller
/// ([`PageDeclParser::parse_value`]).
pub(crate) fn parse_page_size_value<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSize, ParseError<'i, ()>> {
    // Alternative 2, `auto`, tried first via `try_parse` so a non-match
    // rewinds cleanly (same `try_parse`-before-length ordering as
    // `parse_margin_side` in property.rs, and for the same reason: the
    // length branch below unconditionally consumes a token on its first
    // step, so trying it before `auto` without a rewind would eat the
    // `auto` ident and never reach this branch).
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(PageSize::Auto);
    }

    // Alternative 1, `<length>{1,2}`. `parse_page_length` enforces both the
    // "`<length>`, not `<length-percentage>`" grammar restriction and the
    // "Negative lengths are illegal" prose constraint.
    if let Ok(first) = input.try_parse(parse_page_length) {
        let second = input.try_parse(parse_page_length).ok();
        return Ok(match second {
            Some(height) => PageSize::Lengths {
                width: first,
                height,
            },
            // "If only one length value is specified, it sets both the
            // width and height of the page box."
            None => PageSize::Lengths {
                width: first,
                height: first,
            },
        });
    }

    // Alternative 3, `[ <page-size> || [ portrait | landscape ] ]`. `||`
    // means each sub-component is independently optional but at least one
    // must be present, in either source order — so both are tried,
    // repeatedly, until neither matches (each can only match once: once
    // `keyword`/`orientation` is `Some`, that branch is skipped, so a
    // second `A4 A4` or `landscape landscape` does not loop forever nor
    // silently accept the duplicate).
    let mut keyword = None;
    let mut orientation = None;
    loop {
        if keyword.is_none()
            && let Ok(kw) = input.try_parse(parse_page_size_keyword)
        {
            keyword = Some(kw);
            continue;
        }
        if orientation.is_none()
            && let Ok(o) = input.try_parse(parse_page_orientation)
        {
            orientation = Some(o);
            continue;
        }
        break;
    }
    match (keyword, orientation) {
        // Neither sub-component matched — none of the three alternatives
        // apply, so the whole `size` declaration is invalid.
        (None, None) => Err(input.new_custom_error(())),
        (keyword, orientation) => Ok(PageSize::Named {
            keyword,
            orientation,
        }),
    }
}

/// One `<length>` component of the `size` descriptor's `<length>{1,2}`
/// alternative (CSS Paged Media Level 3 §7.1) — thin `Result`-returning
/// wrapper around [`parse_non_negative_length`] so it can be passed directly
/// as a function pointer to `Parser::try_parse` (both length components in
/// [`parse_page_size_value`] share this one parser).
pub(crate) fn parse_page_length<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_non_negative_length(input).ok_or_else(|| input.new_custom_error(()))
}

/// Parse a single `<page-size>` keyword. Grammar: `A5 | A4 | A3 | B5 | B4 |
/// JIS-B5 | JIS-B4 | letter | legal | ledger` — CSS Paged Media Level 3 §7.1
/// "Page size: the size property"
/// (<https://www.w3.org/TR/css-page-3/#page-size-prop>). ASCII
/// case-insensitive, matching every other keyword production in this module.
pub(crate) fn parse_page_size_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSizeKeyword, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    let keyword = match_ignore_ascii_case! { &ident,
        "a5" => PageSizeKeyword::A5,
        "a4" => PageSizeKeyword::A4,
        "a3" => PageSizeKeyword::A3,
        "b5" => PageSizeKeyword::B5,
        "b4" => PageSizeKeyword::B4,
        "jis-b5" => PageSizeKeyword::JisB5,
        "jis-b4" => PageSizeKeyword::JisB4,
        "letter" => PageSizeKeyword::Letter,
        "legal" => PageSizeKeyword::Legal,
        "ledger" => PageSizeKeyword::Ledger,
        _ => return Err(input.new_custom_error(())),
    };
    Ok(keyword)
}

/// Parse a single `portrait | landscape` orientation keyword — the second
/// half of the `size` descriptor's third grammar alternative (CSS Paged
/// Media Level 3 §7.1, see [`parse_page_size_keyword`] for the shared
/// anchor). ASCII case-insensitive.
pub(crate) fn parse_page_orientation<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageOrientation, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    let orientation = match_ignore_ascii_case! { &ident,
        "portrait" => PageOrientation::Portrait,
        "landscape" => PageOrientation::Landscape,
        _ => return Err(input.new_custom_error(())),
    };
    Ok(orientation)
}

/// Parse the `marks` descriptor's value grammar — CSS Paged Media Level 3
/// §7.2 "Crop and Registration Marks: the marks property"
/// (<https://www.w3.org/TR/css-page-3/#marks>), grammar (spec verbatim):
/// `none | [ crop || cross ]`. See [`PageMarks`] for the parsed shape.
///
/// `none` is tried first via `try_parse` (same rewind-on-mismatch ordering
/// [`parse_page_size_value`] uses for its `auto` alternative) since it is a
/// distinct top-level alternative, not a possible outcome of the `||`
/// combinator below.
///
/// The `||` combinator on the second alternative is handled with the same
/// loop-until-neither-matches idiom [`parse_page_size_value`] uses for its
/// own `<page-size> || [ portrait | landscape ]` alternative: `crop` and
/// `cross` are each tried at most once (so `crop crop` doesn't loop forever
/// nor silently accept the duplicate — the second `crop` is left as
/// unconsumed trailing garbage for the caller's `expect_exhausted` to
/// reject), in either source order, and at least one of the two must match
/// or the whole declaration is invalid.
///
/// Called with the parser positioned right after `marks` `:` (the same shape
/// [`crate::property::parse_value`] callers expect) — consumes exactly the
/// value tokens, leaving any trailing `!important` for the caller
/// ([`PageDeclParser::parse_value`]).
pub(crate) fn parse_page_marks_value<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageMarks, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("none"))
        .is_ok()
    {
        return Ok(PageMarks::None);
    }

    let mut crop = false;
    let mut cross = false;
    loop {
        if !crop
            && input
                .try_parse(|input| input.expect_ident_matching("crop"))
                .is_ok()
        {
            crop = true;
            continue;
        }
        if !cross
            && input
                .try_parse(|input| input.expect_ident_matching("cross"))
                .is_ok()
        {
            cross = true;
            continue;
        }
        break;
    }
    if !crop && !cross {
        // Neither alternative matched — `none` failed above, and the `||`
        // combinator requires at least one of `crop` / `cross`.
        return Err(input.new_custom_error(()));
    }
    Ok(PageMarks::Marks { crop, cross })
}

/// Parse the `bleed` descriptor's value grammar — CSS Paged Media Level 3
/// §7.3 "Bleed Area: the bleed property"
/// (<https://www.w3.org/TR/css-page-3/#bleed>), grammar (spec verbatim):
/// `auto | <length>`. See [`PageBleed`] for the parsed shape.
///
/// `auto` is tried first via `try_parse` — same rewind-then-fall-through
/// ordering [`parse_page_size_value`] uses, needed for the same reason: the
/// length branch unconditionally consumes a token on its first step.
///
/// Unlike `size`'s `<length>` alternative ("Negative lengths are illegal"),
/// `bleed`'s explicitly permits negative values ("Values may be negative,
/// but there may be implementation-specific limits") — so this calls
/// [`parse_length_allow_negative`] rather than [`parse_non_negative_length`]
/// (the `size` descriptor's helper — see [`PageBleed::Length`]'s doc).
///
/// Called with the parser positioned right after `bleed` `:` — consumes
/// exactly the value tokens, leaving any trailing `!important` for the
/// caller ([`PageDeclParser::parse_value`]).
pub(crate) fn parse_page_bleed_value<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageBleed, ParseError<'i, ()>> {
    if input
        .try_parse(|input| input.expect_ident_matching("auto"))
        .is_ok()
    {
        return Ok(PageBleed::Auto);
    }
    let length = parse_length_allow_negative(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(PageBleed::Length(length))
}
