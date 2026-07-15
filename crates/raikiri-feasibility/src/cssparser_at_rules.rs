//! cssparser custom at-rules, nzv-spike for raikiri-spike-nzv.9
//!
//! Verifies that cssparser 0.37's `AtRuleParser` trait can recognise and
//! dispatch the paged-media / print core at-rules required by the raikiri
//! design doc section 4 (CSS pipeline):
//!
//!   * `@page`           — the paged-media entry point.
//!   * `@counter-style`  — needed for list/counter rendering in print.
//!   * `@font-face`      — needed to resolve font resources for paged output.
//!
//! The spike implements a *minimal* parser that classifies each at-rule by
//! name, discards its prelude+body (records only that it was seen), and
//! returns a lightweight `RecognisedAtRule` value from `parse_block`.
//! Correctness of the CSS grammar for each at-rule is out of scope for M0;
//! the goal is only to prove the trait surface supports custom dispatch.

use cssparser::{
    AtRuleParser, CowRcStr, ParseError, Parser, ParserInput, ParserState, QualifiedRuleParser,
    StyleSheetParser,
};

/// The kind of at-rule the spike parser recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtRuleKind {
    /// `@page` — paged media.
    Page,
    /// `@counter-style` — CSS counter styles.
    CounterStyle,
    /// `@font-face` — font-face descriptors.
    FontFace,
}

/// Prelude passes the recognised kind through to `parse_block`.
#[derive(Debug, Clone, Copy)]
pub struct RecognisedPrelude {
    pub kind: AtRuleKind,
}

/// Finished at-rule value produced by the spike parser.
#[derive(Debug, Clone)]
pub struct RecognisedAtRule {
    pub kind: AtRuleKind,
    /// Raw span of the block body as a string (for evidence only; not parsed).
    pub raw_body: String,
}

/// Custom error type carried through `ParseError<'i, SpikeError>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpikeError {
    UnknownAtRule,
}

/// The spike at-rule parser.
pub struct SpikeParser;

impl<'i> AtRuleParser<'i> for SpikeParser {
    type Prelude = RecognisedPrelude;
    type AtRule = RecognisedAtRule;
    type Error = SpikeError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let kind = if name.eq_ignore_ascii_case("page") {
            AtRuleKind::Page
        } else if name.eq_ignore_ascii_case("counter-style") {
            AtRuleKind::CounterStyle
        } else if name.eq_ignore_ascii_case("font-face") {
            AtRuleKind::FontFace
        } else {
            return Err(input.new_custom_error(SpikeError::UnknownAtRule));
        };

        // Discard the prelude tokens: for `@page` there may be a page selector,
        // for the others there is either an ident (`@counter-style disc`) or
        // nothing (`@font-face`).  We do not care about their content in M0.
        while input.next().is_ok() {}

        Ok(RecognisedPrelude { kind })
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        // Consume the block; capture the raw slice for evidence.
        let block_start = input.position();
        while input.next().is_ok() {}
        let raw_body = input.slice_from(block_start).to_owned();
        Ok(RecognisedAtRule {
            kind: prelude.kind,
            raw_body,
        })
    }
}

// `StyleSheetParser::new` requires both traits; provide a rejecting stub
// for qualified rules so top-level style rules are skipped.
impl<'i> QualifiedRuleParser<'i> for SpikeParser {
    type Prelude = ();
    type QualifiedRule = RecognisedAtRule;
    type Error = SpikeError;
    // default methods reject; that's fine for this spike.
}

/// Parse a CSS string with `SpikeParser`, returning the recognised at-rules.
pub fn parse_recognised(css: &str) -> Vec<RecognisedAtRule> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut spike = SpikeParser;
    let iter = StyleSheetParser::new(&mut parser, &mut spike);
    iter.filter_map(|r| r.ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPIKE_CSS: &str = r#"
        @page {
            margin: 1cm;
        }

        @counter-style thumbs {
            system: cyclic;
            symbols: "\1F44D";
            suffix: " ";
        }

        @font-face {
            font-family: raikiri-placeholder;
            src: local("Nonexistent");
        }
    "#;

    #[test]
    fn dispatches_all_three_at_rules() {
        let rules = parse_recognised(SPIKE_CSS);

        // Evidence: exactly three at-rules recognised, in source order,
        // each dispatched to its expected kind.
        assert_eq!(rules.len(), 3, "recognised rules: {:#?}", rules);
        assert_eq!(rules[0].kind, AtRuleKind::Page);
        assert_eq!(rules[1].kind, AtRuleKind::CounterStyle);
        assert_eq!(rules[2].kind, AtRuleKind::FontFace);

        // Body span capture is non-empty for each rule (parse_block reached).
        for r in &rules {
            assert!(
                !r.raw_body.is_empty(),
                "expected non-empty body for {:?}",
                r.kind
            );
        }
    }

    #[test]
    fn unknown_at_rule_is_skipped() {
        // `@media` is not one of the three the spike wants; it should be
        // rejected by parse_prelude and yielded as an Err by StyleSheetParser.
        let css = "@media print { @page {} } @page {}";
        let rules = parse_recognised(css);
        // The trailing `@page {}` must still dispatch even though the earlier
        // `@media` was rejected.
        assert!(
            rules.iter().any(|r| r.kind == AtRuleKind::Page),
            "expected trailing @page to be recognised: {:#?}",
            rules
        );
    }
}
