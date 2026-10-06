//! Media-query context, and the Media Queries 4 parser and evaluator used by
//! the cascade.

use std::sync::Arc;

use cssparser::{ParseError, Parser, ParserInput, Token, match_ignore_ascii_case};

use crate::computed::INITIAL_FONT_SIZE_PX;
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
            // WPT's print-media tests use a nominal 5in × 3in sheet with
            // 0.5in margins, leaving a 4in × 2in page area.
            MediaType::Print => (384, 192),
            MediaType::Screen => (800, 600),
        };
        Self {
            media_type,
            viewport_width,
            viewport_height,
        }
    }

    /// Create a context with an explicit viewport in CSS pixels.
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

    /// Return the viewport width in CSS pixels.
    pub const fn viewport_width(self) -> u32 {
        self.viewport_width
    }

    /// Return the viewport height in CSS pixels.
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

/// Kleene's three-valued AND (Media Queries 4 §3.1); `None` is "unknown".
const fn kleene_and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

const fn kleene_not(value: Option<bool>) -> Option<bool> {
    match value {
        Some(value) => Some(!value),
        None => None,
    }
}

const fn kleene_or(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    kleene_not(kleene_and(kleene_not(left), kleene_not(right)))
}

/// A boolean condition over leaves of type `L`, evaluated with three-valued
/// logic.
///
/// `Unknown` stands for a `<general-enclosed>` term or a feature this engine
/// does not evaluate; MQ4 §3.2 gives both the value "unknown".
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Condition<L> {
    Not(Box<Self>),
    And(Vec<Self>),
    Or(Vec<Self>),
    Leaf(L),
    Unknown,
}

impl<L> Condition<L> {
    fn all(head: Self, rest: Vec<Self>) -> Self {
        if rest.is_empty() {
            head
        } else {
            Self::And(std::iter::once(head).chain(rest).collect())
        }
    }

    fn any(head: Self, rest: Vec<Self>) -> Self {
        if rest.is_empty() {
            head
        } else {
            Self::Or(std::iter::once(head).chain(rest).collect())
        }
    }

    fn eval(&self, leaf: &impl Fn(&L) -> Option<bool>) -> Option<bool> {
        match self {
            Self::Not(inner) => kleene_not(inner.eval(leaf)),
            // `None` is a third truth value here, not an early exit.
            Self::And(terms) => terms
                .iter()
                .map(|term| term.eval(leaf))
                .reduce(kleene_and)
                .unwrap_or(Some(true)),
            Self::Or(terms) => terms
                .iter()
                .map(|term| term.eval(leaf))
                .reduce(kleene_or)
                .unwrap_or(Some(false)),
            Self::Leaf(value) => leaf(value),
            Self::Unknown => None,
        }
    }

    /// Every value this condition can take, assuming each leaf can be either
    /// true or false.
    fn outcomes(&self) -> Outcomes {
        match self {
            Self::Not(inner) => inner.outcomes().map(kleene_not),
            Self::And(terms) => terms.iter().fold(Outcomes::TRUE, |acc, term| {
                acc.zip(term.outcomes(), kleene_and)
            }),
            Self::Or(terms) => terms.iter().fold(Outcomes::FALSE, |acc, term| {
                acc.zip(term.outcomes(), kleene_or)
            }),
            Self::Leaf(_) => Outcomes::EITHER,
            Self::Unknown => Outcomes::UNKNOWN,
        }
    }
}

/// A set of three-valued results, used to drop queries that can never match.
///
/// [`parse_media_prelude`] returns `None` for a list that matches in no
/// context, and callers skip such `@media` blocks entirely, so a query that is
/// always false or unknown has to be recognised while parsing.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Outcomes(u8);

impl Outcomes {
    const VALUES: [Option<bool>; 3] = [Some(true), Some(false), None];
    const TRUE: Self = Self(1);
    const FALSE: Self = Self(2);
    const UNKNOWN: Self = Self(4);
    const EITHER: Self = Self(1 | 2);

    const fn of(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::TRUE,
            Some(false) => Self::FALSE,
            None => Self::UNKNOWN,
        }
    }

    fn contains(self, value: Option<bool>) -> bool {
        self.0 & Self::of(value).0 != 0
    }

    fn values(self) -> impl Iterator<Item = Option<bool>> {
        Self::VALUES
            .into_iter()
            .filter(move |value| self.contains(*value))
    }

    /// Apply `op` to every pair of values from `self` and `other`.
    fn zip(self, other: Self, op: impl Fn(Option<bool>, Option<bool>) -> Option<bool>) -> Self {
        let mut result = Self(0);
        for left in self.values() {
            for right in other.values() {
                result.0 |= Self::of(op(left, right)).0;
            }
        }
        result
    }

    fn map(self, op: impl Fn(Option<bool>) -> Option<bool>) -> Self {
        self.zip(Self::TRUE, |value, _| op(value))
    }
}

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
    let list = parse_media_query_list(&mut parser).ok()?;
    (!list.is_empty()).then(|| MediaCondition {
        lists: Arc::from([list]),
    })
}

// ---------------------------------------------------------------------------
// Grammar (Media Queries 4 §3)
//
// One function per production; each quotes the production it parses. The
// helpers below stand for the grammar's combinators: `first_of` is `|`,
// `optional` is `?`, `zero_or_more` is `*`, and `parens` is `( ... )`.
// ---------------------------------------------------------------------------

type PResult<'i, T> = Result<T, ParseError<'i, ()>>;
type Production<'i, T> = fn(&mut Parser<'i, '_>) -> PResult<'i, T>;

/// `A | B | ...`: the first alternative that parses.
fn first_of<'i, T>(
    input: &mut Parser<'i, '_>,
    alternatives: &[Production<'i, T>],
) -> PResult<'i, T> {
    let mut error = input.new_custom_error(());
    for alternative in alternatives {
        match input.try_parse(|input| alternative(input)) {
            Ok(value) => return Ok(value),
            Err(err) => error = err,
        }
    }
    Err(error)
}

/// `A?`
fn optional<'i, T>(input: &mut Parser<'i, '_>, production: Production<'i, T>) -> Option<T> {
    input.try_parse(|input| production(input)).ok()
}

/// `A*`
fn zero_or_more<'i, T>(input: &mut Parser<'i, '_>, production: Production<'i, T>) -> Vec<T> {
    std::iter::from_fn(|| optional(input, production)).collect()
}

/// `( A )`: a parenthesised block whose contents are exactly `A`.
fn parens<'i, T>(input: &mut Parser<'i, '_>, production: Production<'i, T>) -> PResult<'i, T> {
    input.expect_parenthesis_block()?;
    input.parse_nested_block(production)
}

/// A keyword, matched ASCII case-insensitively.
fn keyword<'i>(input: &mut Parser<'i, '_>, name: &'static str) -> PResult<'i, ()> {
    Ok(input.expect_ident_matching(name)?)
}

/// `<media-query-list> = <media-query>#`
///
/// A malformed `<media-query>` becomes `not all` (MQ4 §3.2) and is dropped
/// together with queries that can never match. An empty entry or a tokenizer
/// error token invalidates the whole list instead.
fn parse_media_query_list<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, Vec<MediaQuery>> {
    let queries = input.parse_comma_separated(|input| {
        if input.is_exhausted() {
            return Err(input.new_custom_error(()));
        }
        let query = optional(input, parse_media_query);
        // Whatever the query did not consume, including all of it when it
        // failed, is skipped here; an error token anywhere invalidates the
        // list. A parsed query can only have consumed error tokens inside
        // `<general-enclosed>`, which rejects them as well.
        input.expect_no_error_token()?;
        Ok(query)
    })?;
    Ok(queries
        .into_iter()
        .flatten()
        .filter(MediaQuery::can_match)
        .collect())
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
            |input| input.parse_entirely(parse_typed_media_query),
            |input| {
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
            &[
                |input| keyword(input, "not").map(|()| true),
                |input| keyword(input, "only").map(|()| false),
            ],
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
        &[parse_media_not, |input| {
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
        &[parse_media_not, |input| {
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
            |input| parens(input, parse_media_feature),
            |input| parens(input, parse_media_condition),
            |input| parse_general_enclosed(input).map(|()| Condition::Unknown),
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
            |input| parse_mf_plain(input).map(Condition::Leaf),
            |input| parse_mf_range(input),
            |input| parse_mf_boolean(input).map(Condition::Leaf),
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
            |input| {
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
            |input| {
                let axis = parse_mf_name(input)?;
                let cmp = parse_mf_comparison(input)?;
                let px = parse_mf_value(input)?;
                Ok(Condition::Leaf(ViewportFeature { axis, cmp, px }))
            },
            |input| {
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
        &[parse_mf_lt_or_gt, |input| {
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
