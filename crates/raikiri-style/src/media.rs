//! Media-query context, and the Media Queries 4 parser and evaluator used by
//! the cascade.

use std::sync::Arc;

use cssparser::{Parser, ParserInput, Token, match_ignore_ascii_case};

use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::condition::{Condition, Outcomes, kleene_and, kleene_not};
use crate::grammar::{PResult, first_of, keyword, optional, parens, zero_or_more};
use crate::property::{Length, parse_length_allow_negative};
use crate::resolve::{ComputedLength, ResolveContext, resolve_length};

/// The media type selected for a cascade evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    /// Paged output.
    Print,
    /// Screen output.
    Screen,
}

/// The environment in which a stylesheet is evaluated.
///
/// The initial context is [`MediaContext::print`], matching the renderer's
/// paged-output default. Callers rendering a screen context should pass
/// [`MediaContext::screen`] to [`crate::cascade_with_media_context`].
///
/// Width and height describe the externally selected viewport for screen media
/// and the page box, including margins, for print media (Media Queries 4 §4.1–4.2).
/// The environment stays fixed while evaluating a document: authored `@page`
/// sizes, margins, and named-page geometry do not change these dimensions.
/// CSS Paged Media 3 §7.1 defines the media-query basis as the paper selected
/// without authored `@page` rules.
///
/// The default print environment is a nominal 5in × 3in page box (480 × 288 CSS
/// pixels). Consumers selecting another paper size should supply its dimensions
/// with [`MediaContext::with_viewport`]; layout page defaults are independent.
/// See <https://www.w3.org/TR/mediaqueries-4/#width> and
/// <https://www.w3.org/TR/css-page-3/#page-size>.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaContext {
    media_type: MediaType,
    viewport_width: u32,
    viewport_height: u32,
}

impl MediaContext {
    /// Create a context for the given media type with its default viewport.
    pub const fn new(media_type: MediaType) -> Self {
        let (viewport_width, viewport_height) = match media_type {
            // WPT's nominal print sheet is 5in × 3in. Media queries include
            // page margins rather than using the smaller page area.
            MediaType::Print => (480, 288),
            MediaType::Screen => (800, 600),
        };
        Self {
            media_type,
            viewport_width,
            viewport_height,
        }
    }

    /// Create a context with explicit output dimensions in CSS pixels.
    ///
    /// For print media, pass the externally selected page-box size before
    /// subtracting margins. For screen media, pass the viewport size.
    pub const fn with_viewport(media_type: MediaType, width: u32, height: u32) -> Self {
        Self {
            media_type,
            viewport_width: width,
            viewport_height: height,
        }
    }

    /// Return the default paged-output context.
    pub const fn print() -> Self {
        Self::new(MediaType::Print)
    }

    /// Return a screen-output context.
    pub const fn screen() -> Self {
        Self::new(MediaType::Screen)
    }

    /// Return the selected media type.
    pub const fn media_type(self) -> MediaType {
        self.media_type
    }

    /// Return the viewport or print page-box width in CSS pixels.
    pub const fn viewport_width(self) -> u32 {
        self.viewport_width
    }

    /// Return the viewport or print page-box height in CSS pixels.
    pub const fn viewport_height(self) -> u32 {
        self.viewport_height
    }
}

impl Default for MediaContext {
    fn default() -> Self {
        Self::print()
    }
}

const PRINT_MEDIA: u8 = 1;
const SCREEN_MEDIA: u8 = 2;
const ALL_MEDIA: u8 = PRINT_MEDIA | SCREEN_MEDIA;

const fn media_bit(media_type: MediaType) -> u8 {
    match media_type {
        MediaType::Print => PRINT_MEDIA,
        MediaType::Screen => SCREEN_MEDIA,
    }
}

// ---------------------------------------------------------------------------
// Syntax tree and evaluation
// ---------------------------------------------------------------------------

/// The viewport dimension a range feature tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Width,
    Height,
}

/// A range comparison, read as `<viewport value> <cmp> <px>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Cmp {
    /// The same comparison with its operands swapped: `a < b` is `b > a`.
    const fn flipped(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::Le => Self::Ge,
            Self::Eq => Self::Eq,
            Self::Ge => Self::Le,
            Self::Gt => Self::Lt,
        }
    }
}

/// A `width` / `height` test, normalised from the plain, boolean, and range
/// forms of MQ4 §4.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewportFeature {
    axis: Axis,
    cmp: Cmp,
    px: f32,
}

impl ViewportFeature {
    fn matches(self, context: &MediaContext) -> bool {
        let value = match self.axis {
            Axis::Width => context.viewport_width,
            Axis::Height => context.viewport_height,
        } as f32;
        match self.cmp {
            Cmp::Lt => value < self.px,
            Cmp::Le => value <= self.px,
            Cmp::Eq => value == self.px,
            Cmp::Ge => value >= self.px,
            Cmp::Gt => value > self.px,
        }
    }
}

type FeatureCondition = Condition<ViewportFeature>;

/// One entry of a `<media-query-list>`.
#[derive(Clone, Debug, PartialEq)]
enum MediaQuery {
    /// `<media-condition>`
    Condition(FeatureCondition),
    /// `[ not | only ]? <media-type> [ and <media-condition-without-or> ]?`
    Typed {
        negated: bool,
        media_mask: u8,
        condition: Option<FeatureCondition>,
    },
}

impl MediaQuery {
    /// Evaluate the query. An unknown result is `not all` (MQ4 §3.2).
    fn matches(&self, context: &MediaContext) -> bool {
        let feature = |feature: &ViewportFeature| Some(feature.matches(context));
        let value = match self {
            Self::Condition(condition) => condition.eval(&feature),
            Self::Typed {
                negated,
                media_mask,
                condition,
            } => {
                let type_matches = Some(media_mask & media_bit(context.media_type) != 0);
                let condition = condition.as_ref().map_or(Some(true), |c| c.eval(&feature));
                let value = kleene_and(type_matches, condition);
                if *negated { kleene_not(value) } else { value }
            }
        };
        value == Some(true)
    }

    fn can_match(&self) -> bool {
        let outcomes = match self {
            Self::Condition(condition) => condition.outcomes(),
            Self::Typed {
                negated,
                media_mask,
                condition,
            } => {
                let type_outcomes = match *media_mask {
                    0 => Outcomes::FALSE,
                    ALL_MEDIA => Outcomes::TRUE,
                    _ => Outcomes::EITHER,
                };
                let condition = condition
                    .as_ref()
                    .map_or(Outcomes::TRUE, Condition::outcomes);
                let outcomes = type_outcomes.zip(condition, kleene_and);
                if *negated {
                    outcomes.map(kleene_not)
                } else {
                    outcomes
                }
            }
        };
        outcomes.contains(Some(true))
    }
}

/// The condition guarding a rule: every nested `@media` list must match.
///
/// Each list holds only the queries that can match in some context; queries
/// that are malformed or always `not all` are dropped while parsing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaCondition {
    lists: Arc<[Vec<MediaQuery>]>,
    paper_dependent: bool,
}

/// A parsed qualified rule that is guarded by a media condition.
///
/// Kept separate from `RuleTree::style_rules` so the historical direct-rule
/// view and its indexes remain unchanged when a stylesheet gains `@media`.
pub(crate) struct MediaRule {
    pub(crate) rule: crate::rule::StyleRule,
    pub(crate) condition: MediaCondition,
}

impl MediaCondition {
    /// Whether a containing query qualifies declarations by paper dimensions.
    /// CSS Paged Media 3 §7.1 excludes only `size` under these conditions.
    pub(crate) fn depends_on_paper_size(&self) -> bool {
        self.paper_dependent
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lists.iter().any(Vec::is_empty)
    }

    pub(crate) fn matches(&self, context: &MediaContext) -> bool {
        self.lists
            .iter()
            .all(|list| list.iter().any(|query| query.matches(context)))
    }

    pub(crate) fn intersect(&self, other: &Self) -> Self {
        Self {
            lists: self
                .lists
                .iter()
                .chain(other.lists.iter())
                .cloned()
                .collect(),
            paper_dependent: self.paper_dependent || other.paper_dependent,
        }
    }
}

/// Parse an `@media` prelude.
///
/// Returns `None` when the list is invalid as a whole (an empty entry or a
/// tokenizer error token) or when none of its queries can match.
pub(crate) fn parse_media_prelude(source: &str) -> Option<MediaCondition> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let (list, paper_dependent) = parse_media_query_list(&mut parser).ok()?;
    (!list.is_empty()).then(|| MediaCondition {
        lists: Arc::from([list]),
        paper_dependent,
    })
}

/// Retain paper-feature provenance before unsupported queries are discarded.
/// A media list can still match through `print` while an orientation query is
/// unknown; that does not remove its qualification of a page `size` descriptor.
fn prelude_depends_on_paper_size(source: &str) -> bool {
    fn paper_feature(name: &str) -> bool {
        match_ignore_ascii_case! { name,
            "width" | "min-width" | "max-width" |
            "height" | "min-height" | "max-height" |
            "device-width" | "min-device-width" | "max-device-width" |
            "device-height" | "min-device-height" | "max-device-height" |
            "aspect-ratio" | "min-aspect-ratio" | "max-aspect-ratio" |
            "device-aspect-ratio" | "min-device-aspect-ratio" | "max-device-aspect-ratio" |
            "orientation" => true,
            _ => false,
        }
    }

    fn comparison(token: &Token<'_>) -> bool {
        matches!(token, Token::Delim('<' | '>' | '='))
    }

    fn scan<'i>(input: &mut Parser<'i, '_>, in_parens: bool) -> PResult<'i, bool> {
        fn finish<'i>(input: &mut Parser<'i, '_>, dependent: bool) -> PResult<'i, bool> {
            input.expect_no_error_token()?;
            Ok(dependent)
        }

        let mut previous_comparison = false;
        let mut first = true;
        let mut value_first = false;
        let mut dependent = false;
        while let Ok(token) = input.next().cloned() {
            if in_parens && first {
                match &token {
                    Token::Ident(name) => {
                        // Plain/boolean/name-first features occupy this whole
                        // block. Their values are opaque, including nested
                        // blocks that resemble another feature or a range.
                        let feature = input
                            .try_parse(|input| {
                                if input.is_exhausted() {
                                    return Ok(());
                                }
                                let location = input.current_source_location();
                                match input.next() {
                                    Ok(Token::Colon) => Ok(()),
                                    Ok(token) if comparison(token) => Ok(()),
                                    _ => Err(location.new_custom_error::<(), ()>(())),
                                }
                            })
                            .is_ok();
                        if feature {
                            return finish(input, paper_feature(name));
                        }
                        if !name.eq_ignore_ascii_case("not") {
                            return finish(input, false);
                        }
                    }
                    Token::Number { .. }
                    | Token::Dimension { .. }
                    | Token::Percentage { .. }
                    | Token::Function(_) => value_first = true,
                    Token::ParenthesisBlock => {}
                    _ => return finish(input, false),
                }
            }
            match &token {
                Token::ParenthesisBlock if !value_first => {
                    dependent |= input.parse_nested_block(|nested| scan(nested, true))?;
                }
                Token::Ident(name) if value_first && previous_comparison => {
                    return finish(input, paper_feature(name));
                }
                _ => {}
            }
            previous_comparison = comparison(&token);
            first = false;
        }
        Ok(dependent)
    }

    let mut input = ParserInput::new(source);
    scan(&mut Parser::new(&mut input), false).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Grammar (Media Queries 4 §3)
//
// One function per production; each quotes the production it parses. The
// shared grammar helpers stand for the grammar's combinators: `first_of` is `|`,
// `optional` is `?`, `zero_or_more` is `*`, and `parens` is `( ... )`.
// ---------------------------------------------------------------------------

/// `<media-query-list> = <media-query>#`
///
/// A malformed `<media-query>` becomes `not all` (MQ4 §3.2) and is dropped
/// together with queries that can never match. An empty entry or a tokenizer
/// error token invalidates the whole list instead.
fn parse_media_query_list<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, (Vec<MediaQuery>, bool)> {
    let queries = input.parse_comma_separated(|input| {
        if input.is_exhausted() {
            return Err(input.new_custom_error(()));
        }
        let start = input.position();
        let query = optional(input, parse_media_query);
        // Invalid arms become `not all` and cannot qualify declarations.
        // Valid unsupported paper features still retain their provenance.
        let paper_dependent =
            query.is_some() && prelude_depends_on_paper_size(input.slice_from(start));
        // Whatever the query did not consume, including all of it when it
        // failed, is skipped here; an error token anywhere invalidates the
        // list. A parsed query can only have consumed error tokens inside
        // `<general-enclosed>`, which rejects them as well.
        input.expect_no_error_token()?;
        Ok((query, paper_dependent))
    })?;
    let paper_dependent = queries.iter().any(|(_, dependent)| *dependent);
    let list = queries
        .into_iter()
        .filter_map(|(query, _)| query)
        .filter(MediaQuery::can_match)
        .collect();
    Ok((list, paper_dependent))
}

/// `<media-query> = <media-condition>
///                | [ not | only ]? <media-type> [ and <media-condition-without-or> ]?`
///
/// The typed form is tried first because it is the common case. No input
/// matches both: the typed form starts with an identifier that is not
/// followed by a parenthesis, a condition with a parenthesis or `not (`.
fn parse_media_query<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, MediaQuery> {
    first_of(
        input,
        &[
            &|input| input.parse_entirely(parse_typed_media_query),
            &|input| {
                input
                    .parse_entirely(parse_media_condition)
                    .map(MediaQuery::Condition)
            },
        ],
    )
}

/// `[ not | only ]? <media-type> [ and <media-condition-without-or> ]?`
fn parse_typed_media_query<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, MediaQuery> {
    let negated = optional(input, |input| {
        first_of(
            input,
            &[&|input| keyword(input, "not").map(|()| true), &|input| {
                keyword(input, "only").map(|()| false)
            }],
        )
    });
    let media_mask = parse_media_type(input)?;
    let condition = optional(input, |input| {
        keyword(input, "and")?;
        parse_media_condition_without_or(input)
    });
    Ok(MediaQuery::Typed {
        negated: negated == Some(true),
        media_mask,
        condition,
    })
}

/// `<media-type> = <ident>`, excluding `only`, `not`, `and`, `or`, and
/// `layer` (MQ4 §3). An unknown media type is valid but matches nothing.
fn parse_media_type<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, u8> {
    let location = input.current_source_location();
    let name = input.expect_ident()?;
    match_ignore_ascii_case! { name,
        "all" => Ok(ALL_MEDIA),
        "print" => Ok(PRINT_MEDIA),
        "screen" => Ok(SCREEN_MEDIA),
        "only" | "not" | "and" | "or" | "layer" => Err(location.new_custom_error(())),
        _ => Ok(0),
    }
}

/// `<media-condition> = <media-not> | <media-in-parens> [ <media-and>* | <media-or>* ]`
fn parse_media_condition<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    first_of(
        input,
        &[&parse_media_not, &|input| {
            let head = parse_media_in_parens(input)?;
            let and = zero_or_more(input, parse_media_and);
            if !and.is_empty() {
                return Ok(Condition::all(head, and));
            }
            Ok(Condition::any(head, zero_or_more(input, parse_media_or)))
        }],
    )
}

/// `<media-condition-without-or> = <media-not> | <media-in-parens> <media-and>*`
fn parse_media_condition_without_or<'i>(
    input: &mut Parser<'i, '_>,
) -> PResult<'i, FeatureCondition> {
    first_of(
        input,
        &[&parse_media_not, &|input| {
            let head = parse_media_in_parens(input)?;
            Ok(Condition::all(head, zero_or_more(input, parse_media_and)))
        }],
    )
}

/// `<media-not> = not <media-in-parens>`
fn parse_media_not<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    keyword(input, "not")?;
    Ok(Condition::Not(Box::new(parse_media_in_parens(input)?)))
}

/// `<media-and> = and <media-in-parens>`
fn parse_media_and<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    keyword(input, "and")?;
    parse_media_in_parens(input)
}

/// `<media-or> = or <media-in-parens>`
fn parse_media_or<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    keyword(input, "or")?;
    parse_media_in_parens(input)
}

/// `<media-in-parens> = ( <media-condition> ) | ( <media-feature> ) | <general-enclosed>`
///
/// `( <media-feature> )` is tried first because it is the common case. No
/// input matches both parenthesised forms: a feature starts with a feature
/// name or a value, a condition with `not` or another parenthesis.
fn parse_media_in_parens<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    first_of(
        input,
        &[
            &|input| parens(input, parse_media_feature),
            &|input| parens(input, parse_media_condition),
            &|input| parse_general_enclosed(input).map(|()| Condition::Unknown),
        ],
    )
}

/// `<general-enclosed> = [ <function-token> <any-value>? ) ] | [ ( <any-value>? ) ]`
///
/// `<any-value>` excludes tokenizer error tokens, so the block contents are
/// rejected if they hold one at any depth.
fn parse_general_enclosed<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, ()> {
    let location = input.current_source_location();
    match input.next()? {
        Token::Function(_) | Token::ParenthesisBlock => {}
        _ => return Err(location.new_custom_error(())),
    }
    input.parse_nested_block(|input| Ok(input.expect_no_error_token()?))
}

/// `<media-feature> = [ <mf-plain> | <mf-boolean> | <mf-range> ]`
///
/// `<mf-boolean>` is tried last because it is a prefix of the other forms.
/// Only `width` and `height` are evaluated; any other feature, or a value of
/// the wrong type, falls through to `<general-enclosed>` and is unknown.
fn parse_media_feature<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    first_of(
        input,
        &[
            &|input| parse_mf_plain(input).map(Condition::Leaf),
            &|input| parse_mf_range(input),
            &|input| parse_mf_boolean(input).map(Condition::Leaf),
        ],
    )
}

/// `<mf-plain> = <mf-name> : <mf-value>`, with the `min-` / `max-` prefixes.
fn parse_mf_plain<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, ViewportFeature> {
    let location = input.current_source_location();
    let name = input.expect_ident()?;
    let (cmp, axis) = match_ignore_ascii_case! { name,
        "width" => (Cmp::Eq, Axis::Width),
        "min-width" => (Cmp::Ge, Axis::Width),
        "max-width" => (Cmp::Le, Axis::Width),
        "height" => (Cmp::Eq, Axis::Height),
        "min-height" => (Cmp::Ge, Axis::Height),
        "max-height" => (Cmp::Le, Axis::Height),
        _ => return Err(location.new_custom_error(())),
    };
    input.expect_colon()?;
    let px = parse_mf_value(input)?;
    Ok(ViewportFeature { axis, cmp, px })
}

/// `<mf-boolean> = <mf-name>`: true when the value is not zero.
fn parse_mf_boolean<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, ViewportFeature> {
    let axis = parse_mf_name(input)?;
    Ok(ViewportFeature {
        axis,
        cmp: Cmp::Gt,
        px: 0.0,
    })
}

/// `<mf-range> = <mf-name> <mf-comparison> <mf-value>
///             | <mf-value> <mf-comparison> <mf-name>
///             | <mf-value> <mf-lt> <mf-name> <mf-lt> <mf-value>
///             | <mf-value> <mf-gt> <mf-name> <mf-gt> <mf-value>`
///
/// The two-sided forms are tried first because the second form is their
/// prefix.
fn parse_mf_range<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, FeatureCondition> {
    first_of(
        input,
        &[
            &|input| {
                let low = parse_mf_value(input)?;
                let first = parse_mf_lt_or_gt(input)?;
                let axis = parse_mf_name(input)?;
                let second = parse_mf_lt_or_gt(input)?;
                let high = parse_mf_value(input)?;
                if matches!(first, Cmp::Lt | Cmp::Le) != matches!(second, Cmp::Lt | Cmp::Le) {
                    return Err(input.new_custom_error(()));
                }
                Ok(Condition::And(vec![
                    Condition::Leaf(ViewportFeature {
                        axis,
                        cmp: first.flipped(),
                        px: low,
                    }),
                    Condition::Leaf(ViewportFeature {
                        axis,
                        cmp: second,
                        px: high,
                    }),
                ]))
            },
            &|input| {
                let axis = parse_mf_name(input)?;
                let cmp = parse_mf_comparison(input)?;
                let px = parse_mf_value(input)?;
                Ok(Condition::Leaf(ViewportFeature { axis, cmp, px }))
            },
            &|input| {
                let px = parse_mf_value(input)?;
                let cmp = parse_mf_comparison(input)?;
                let axis = parse_mf_name(input)?;
                Ok(Condition::Leaf(ViewportFeature {
                    axis,
                    cmp: cmp.flipped(),
                    px,
                }))
            },
        ],
    )
}

/// `<mf-name> = <ident>`, limited to the range features evaluated here.
fn parse_mf_name<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, Axis> {
    let location = input.current_source_location();
    let name = input.expect_ident()?;
    match_ignore_ascii_case! { name,
        "width" => Ok(Axis::Width),
        "height" => Ok(Axis::Height),
        _ => Err(location.new_custom_error(())),
    }
}

/// `<mf-value>`, limited to the `<length>` that `width` and `height` take.
///
/// Relative units resolve against the initial font size (MQ4 §1.3); `lh` and
/// `rlh` need a line height that has no initial length, so they are unknown.
fn parse_mf_value<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, f32> {
    let location = input.current_source_location();
    let initial = ComputedLength(INITIAL_FONT_SIZE_PX);
    match parse_length_allow_negative(input) {
        Some(Length::Lh(_) | Length::Rlh(_)) | None => Err(location.new_custom_error(())),
        Some(length) => Ok(resolve_length(length, initial, None, &ResolveContext::new(initial)).0),
    }
}

/// `<mf-comparison> = <mf-lt> | <mf-gt> | <mf-eq>`
fn parse_mf_comparison<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, Cmp> {
    first_of(
        input,
        &[&parse_mf_lt_or_gt, &|input| {
            input.expect_delim('=')?;
            Ok(Cmp::Eq)
        }],
    )
}

/// `<mf-lt> = '<' '='?` and `<mf-gt> = '>' '='?`, with no space before `=`.
fn parse_mf_lt_or_gt<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, Cmp> {
    let location = input.current_source_location();
    let less = match input.next()? {
        Token::Delim('<') => true,
        Token::Delim('>') => false,
        _ => return Err(location.new_custom_error(())),
    };
    let or_equal = input
        .try_parse(|input| match input.next_including_whitespace() {
            Ok(Token::Delim('=')) => Ok(()),
            _ => Err(()),
        })
        .is_ok();
    Ok(match (less, or_equal) {
        (true, false) => Cmp::Lt,
        (true, true) => Cmp::Le,
        (false, false) => Cmp::Gt,
        (false, true) => Cmp::Ge,
    })
}

#[cfg(test)]
mod tests;
