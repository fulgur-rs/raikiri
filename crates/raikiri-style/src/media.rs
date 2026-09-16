//! Media-query context and the small media-type subset used by the cascade.

use cssparser::{Parser, ParserInput};

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
}

impl MediaContext {
    /// Create a context for the given media type.
    pub const fn new(media_type: MediaType) -> Self {
        Self { media_type }
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
}

impl Default for MediaContext {
    fn default() -> Self {
        Self::print()
    }
}

const PRINT_MEDIA: u8 = 1;
const SCREEN_MEDIA: u8 = 2;
const ALL_MEDIA: u8 = PRINT_MEDIA | SCREEN_MEDIA;

/// A parsed OR-list of supported media types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MediaCondition(u8);

/// A parsed qualified rule that is guarded by a media condition.
///
/// Kept separate from `RuleTree::style_rules` so the historical direct-rule
/// view and its indexes remain unchanged when a stylesheet gains `@media`.
pub(crate) struct MediaRule {
    pub(crate) rule: crate::rule::StyleRule,
    pub(crate) condition: MediaCondition,
}

impl MediaCondition {
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn matches(self, context: &MediaContext) -> bool {
        let bit = match context.media_type {
            MediaType::Print => PRINT_MEDIA,
            MediaType::Screen => SCREEN_MEDIA,
        };
        self.0 & bit != 0
    }

    pub(crate) const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// Parse the intentionally small media-query subset supported by this pass.
///
/// Each comma-separated query is evaluated independently. A query is valid
/// only when it consists of one `all`, `print`, or `screen` identifier. This
/// lets a supported alternative survive beside an unknown one while rejecting
/// feature expressions and malformed alternatives without executing them.
pub(crate) fn parse_media_condition(source: &str) -> Option<MediaCondition> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let alternatives = parser
        .parse_comma_separated::<_, u8, ()>(|query| {
            let start = query.position();
            query
                .expect_no_error_token()
                .map_err(|_| query.new_custom_error(()))?;
            let raw = query.slice(start..query.position());

            let mut alternative_input = ParserInput::new(raw);
            let mut alternative = Parser::new(&mut alternative_input);
            // A comma-separated list must not contain an empty alternative.
            // Unknown but syntactically non-empty media queries are harmless
            // and may coexist with a supported alternative; an empty one
            // makes the whole prelude malformed and must fail closed.
            if alternative.expect_exhausted().is_ok() {
                return Err(query.new_custom_error(()));
            }

            let Ok(name) = alternative.expect_ident().map(|name| name.to_string()) else {
                return Ok(0);
            };
            if alternative.expect_exhausted().is_err() {
                return Ok(0);
            }
            Ok(match name.to_ascii_lowercase().as_str() {
                "all" => ALL_MEDIA,
                "print" => PRINT_MEDIA,
                "screen" => SCREEN_MEDIA,
                _ => 0,
            })
        })
        .ok()?;
    let mask = alternatives
        .into_iter()
        .fold(0, |mask, alternative| mask | alternative);
    (mask != 0).then_some(MediaCondition(mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_constructors_and_default() {
        assert_eq!(MediaContext::default(), MediaContext::print());
        assert_eq!(MediaContext::print().media_type(), MediaType::Print);
        assert_eq!(MediaContext::screen().media_type(), MediaType::Screen);
        assert_eq!(MediaContext::new(MediaType::Screen), MediaContext::screen());
    }

    #[test]
    fn parses_supported_media_alternatives() {
        let condition = parse_media_condition("print, projection, screen").unwrap();
        assert!(condition.matches(&MediaContext::print()));
        assert!(condition.matches(&MediaContext::screen()));
        assert_eq!(parse_media_condition("projection"), None);
    }

    #[test]
    fn rejects_features_and_extra_tokens() {
        assert_eq!(parse_media_condition("screen and (color)"), None);
        assert_eq!(parse_media_condition("not print"), None);
        assert_eq!(parse_media_condition("print screen"), None);
        assert!(parse_media_condition("screen, (min-width: 1px), print").is_some());
    }

    #[test]
    fn nested_commas_and_tokenizer_errors_do_not_leak_supported_names() {
        assert_eq!(parse_media_condition("projection(foo, screen"), None);
        assert_eq!(parse_media_condition("print, url(\"bad\n\")"), None);
        let condition = parse_media_condition("print, (min-width: 1px, 2px)").unwrap();
        assert!(condition.matches(&MediaContext::print()));
    }

    #[test]
    fn rejects_empty_media_alternatives() {
        assert_eq!(parse_media_condition("print,"), None);
        assert_eq!(parse_media_condition(", print"), None);
        assert_eq!(parse_media_condition("print,,screen"), None);
        assert_eq!(parse_media_condition("print, /* comment */"), None);
    }

    #[test]
    fn comments_and_escaped_names_are_tokenized() {
        let condition = parse_media_condition(" /* before */ \\70 rint /*,*/ ").unwrap();
        assert!(condition.matches(&MediaContext::print()));
    }
}
