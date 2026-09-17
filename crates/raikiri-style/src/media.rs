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

/// A parsed media query with the small media-type and viewport-feature subset
/// used by the cascade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MediaCondition {
    media_mask: u8,
    min_width: Option<u32>,
    max_width: Option<u32>,
    min_height: Option<u32>,
    max_height: Option<u32>,
}

/// A parsed qualified rule that is guarded by a media condition.
///
/// Kept separate from `RuleTree::style_rules` so the historical direct-rule
/// view and its indexes remain unchanged when a stylesheet gains `@media`.
pub(crate) struct MediaRule {
    pub(crate) rule: crate::rule::StyleRule,
    pub(crate) condition: MediaCondition,
}

const fn max_bound(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left > right { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

const fn min_bound(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left < right { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

impl MediaCondition {
    pub(crate) const fn is_empty(self) -> bool {
        self.media_mask == 0
            || matches!((self.min_width, self.max_width), (Some(min), Some(max)) if min > max)
            || matches!((self.min_height, self.max_height), (Some(min), Some(max)) if min > max)
    }

    pub(crate) const fn matches(self, context: &MediaContext) -> bool {
        let bit = match context.media_type {
            MediaType::Print => PRINT_MEDIA,
            MediaType::Screen => SCREEN_MEDIA,
        };
        if self.media_mask & bit == 0 {
            return false;
        }
        let width = context.viewport_width;
        let height = context.viewport_height;
        if let Some(min) = self.min_width
            && width < min
        {
            return false;
        }
        if let Some(max) = self.max_width
            && width > max
        {
            return false;
        }
        if let Some(min) = self.min_height
            && height < min
        {
            return false;
        }
        if let Some(max) = self.max_height
            && height > max
        {
            return false;
        }
        true
    }

    pub(crate) const fn intersect(self, other: Self) -> Self {
        Self {
            media_mask: self.media_mask & other.media_mask,
            min_width: max_bound(self.min_width, other.min_width),
            max_width: min_bound(self.max_width, other.max_width),
            min_height: max_bound(self.min_height, other.min_height),
            max_height: min_bound(self.max_height, other.max_height),
        }
    }
}

fn parse_media_length(value: &str) -> Option<u32> {
    let value = value.trim().to_ascii_lowercase();
    let number_end = value
        .find(|character: char| {
            !(character.is_ascii_digit() || matches!(character, '+' | '-' | '.'))
        })
        .unwrap_or(value.len());
    let number = value[..number_end].parse::<f64>().ok()?;
    let unit = &value[number_end..];
    let multiplier = match unit {
        "" | "px" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "q" => 96.0 / 101.6,
        _ => return None,
    };
    let pixels = number * multiplier;
    (pixels.is_finite() && pixels >= 0.0 && pixels <= u32::MAX as f64)
        .then_some(pixels.round() as u32)
}

fn parse_feature_query(source: &str) -> Option<MediaCondition> {
    if source.contains(',') {
        return None;
    }
    let parts: Vec<_> = source.split("and").map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    let mut media_mask = ALL_MEDIA;
    let mut index = 0;
    if !parts[0].starts_with('(') {
        media_mask = match parts[0].to_ascii_lowercase().as_str() {
            "all" => ALL_MEDIA,
            "print" => PRINT_MEDIA,
            "screen" => SCREEN_MEDIA,
            _ => return None,
        };
        index = 1;
    }
    let mut condition = MediaCondition {
        media_mask,
        min_width: None,
        max_width: None,
        min_height: None,
        max_height: None,
    };
    let mut saw_feature = false;
    for part in &parts[index..] {
        let inner = part.strip_prefix('(')?.strip_suffix(')')?;
        let (name, value) = inner.split_once(':')?;
        let pixels = parse_media_length(value)?;
        match name.trim().to_ascii_lowercase().as_str() {
            "width" => {
                condition.min_width = Some(pixels);
                condition.max_width = Some(pixels);
            }
            "height" => {
                condition.min_height = Some(pixels);
                condition.max_height = Some(pixels);
            }
            "min-width" => condition.min_width = Some(pixels),
            "max-width" => condition.max_width = Some(pixels),
            "min-height" => condition.min_height = Some(pixels),
            "max-height" => condition.max_height = Some(pixels),
            _ => return None,
        }
        saw_feature = true;
    }
    saw_feature.then_some(condition)
}

/// Parse the intentionally small media-query subset supported by this pass.
///
/// In addition to `all`, `print`, and `screen`, a single query may contain
/// `min/max-width` and `min/max-height` features with absolute CSS lengths.
/// Comma-separated lists retain the historical media-type-only behavior.
pub(crate) fn parse_media_condition(source: &str) -> Option<MediaCondition> {
    if let Some(condition) = parse_feature_query(source.trim()) {
        return Some(condition);
    }
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
    (mask != 0).then_some(MediaCondition {
        media_mask: mask,
        min_width: None,
        max_width: None,
        min_height: None,
        max_height: None,
    })
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
    fn default_print_viewport_matches_wpt_page_area() {
        let context = MediaContext::print();
        assert_eq!(context.viewport_width(), 384);
        assert_eq!(context.viewport_height(), 192);
    }

    #[test]
    fn parses_viewport_features() {
        let condition = parse_media_condition(
            "(min-width: 4in) and (max-width: 5in) and (min-height: 2in) and (max-height: 3in)",
        )
        .unwrap();
        assert!(condition.matches(&MediaContext::print()));
        assert!(!condition.matches(&MediaContext::with_viewport(MediaType::Screen, 800, 600,)));
    }

    #[test]
    fn parses_exact_viewport_features() {
        let condition = parse_media_condition("(width: 100px) and (height: 100px)").unwrap();
        assert!(condition.matches(&MediaContext::with_viewport(MediaType::Print, 100, 100,)));
        assert!(!condition.matches(&MediaContext::print()));
    }

    #[test]
    fn parses_media_type_with_viewport_features() {
        let condition = parse_media_condition("print and (min-width: 300px)").unwrap();
        assert!(condition.matches(&MediaContext::print()));
        assert!(!condition.matches(&MediaContext::screen()));
    }

    #[test]
    fn rejects_unsupported_viewport_features() {
        assert_eq!(parse_media_condition("(orientation: landscape)"), None);
        assert_eq!(parse_media_condition("screen and (color)"), None);
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
