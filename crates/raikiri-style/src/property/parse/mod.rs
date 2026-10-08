use std::sync::Arc;

use cssparser::{ParseError, Parser, ParserInput, Token};

use super::types::*;

mod box_model;
mod color;
mod common;
mod content;
mod layout;
mod text;
mod visual;

pub(crate) use box_model::*;
pub use color::*;
pub(crate) use common::*;
pub(crate) use content::*;
pub(crate) use layout::*;
pub use text::*;
pub use visual::*;

/// Returns whether `name` (ASCII case-insensitively) is registered as a
/// supported CSS property: any name in [`super::supported_property_names`],
/// including shorthands that fully expand during parsing and so have no
/// `PropertyKey`. Custom properties (`--*`) are not included.
pub fn is_supported_property_name(name: &str) -> bool {
    super::supported_property_names()
        .binary_search(&name.to_ascii_lowercase().as_str())
        .is_ok()
}

/// Produce a `PropertyValue` from a property name and parser.
/// Return `None` for an unknown name or invalid value.
pub fn parse_value(name: &str, input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    if is_custom_property_name(name) {
        let value = consume_deferred_value(input)?;
        return Some(PropertyValue::CustomProperty(CustomProperty {
            name: name.into(),
            value,
        }));
    }

    let key = property_key_for_name(name);
    let normalized_name = name.to_ascii_lowercase();
    let start = input.state();
    // Preserve defaulting markers until inheritance or cascade rollback can
    // supply the correct value. Canonicalize the token so escapes, case, and
    // comments have the same behavior throughout the cascade.
    if matches!(
        key,
        Some(PropertyKey::Color | PropertyKey::BackgroundColor | PropertyKey::FontSize)
    ) && let Ok(keyword) = input.try_parse(|parser| {
        let keyword = parse_css_wide_keyword_res(parser)?;
        Ok::<_, ParseError<'_, ()>>(keyword)
    }) {
        return Some(PropertyValue::Deferred(DeferredValue {
            property: normalized_name.clone().into(),
            value: keyword.as_css_str().into(),
            key: key?,
        }));
    }
    if key.is_some() && contains_deferred_function(input) {
        input.reset(&start);
        let value = consume_deferred_value(input)?;
        let is_color_property = matches!(normalized_name.as_str(), "color" | "background-color");
        if is_color_property && value.to_ascii_lowercase().contains("(from") {
            // Relative colors may use var() as their origin. Validate their
            // complete grammar before deferring; the generic property math
            // path cannot distinguish an invalid channel from a variable.
            if color_value_with_math_is_valid(value.as_ref()) {
                return Some(PropertyValue::Deferred(DeferredValue {
                    property: name.to_ascii_lowercase().into(),
                    value,
                    key: key?,
                }));
            }
            return None;
        }
        if !contains_function_in_source(&value, "var") {
            if normalized_name == "hyphenate-limit-chars" {
                // Integer math is rounded only when it came from a math
                // function. Reparse the original token stream so direct
                // fractional number tokens remain invalid.
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                let parsed = parse_hyphenate_limit_chars(&mut reparsed)?;
                if reparsed.expect_exhausted().is_ok() {
                    return Some(PropertyValue::HyphenateLimitChars(parsed));
                }
                return None;
            }
            // `text-indent` keeps its simple additive calc terms instead of
            // reducing mixed units to the generic deferred fallback. This
            // also lets `hanging` / `each-line` stay attached to the value.
            if normalized_name == "text-indent" {
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_text_indent(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::TextIndent(parsed));
                }
            }
            // `letter-spacing` also keeps a simple mixed calc instead of
            // reducing it through the generic deferred-value path.
            if normalized_name == "letter-spacing" {
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_letter_spacing(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::LetterSpacing(parsed));
                }
            }
            if normalized_name == "word-spacing" {
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_word_spacing(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::WordSpacing(parsed));
                }
            }
            // `tab-size` keeps its simple additive length calc instead of
            // reducing it through the generic deferred-value path
            // (see the "Handling `calc()`" section of `parse_tab_size`:
            // percentages are rejected because Percentages: N/A, while forms with
            // `sign()` or container-relative units remain dropped).
            if normalized_name == "tab-size" {
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_tab_size(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::TabSize(parsed));
                }
            }
            if normalized_name == "text-underline-offset" {
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_text_underline_offset(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::TextUnderlineOffset(parsed));
                }
            }
            if normalized_name == "text-shadow"
                && !contains_function_in_source(value.as_ref(), "var")
            {
                if math_source_has_percentage(value.as_ref()) {
                    return None;
                }
                let mut reparsed_input = ParserInput::new(value.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_text_shadow(&mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(PropertyValue::TextShadow(if parsed.is_empty() {
                        empty_text_shadow_list()
                    } else {
                        Arc::new(parsed)
                    }));
                }
            }
            if is_color_property {
                // Parse the original color grammar so each calc() keeps its
                // position-specific numeric type. This covers ordinary color
                // math without the false positives caused by an untyped dummy.
                if color_value_with_math_is_valid(value.as_ref()) {
                    return Some(PropertyValue::Deferred(DeferredValue {
                        property: name.to_ascii_lowercase().into(),
                        value,
                        key: key?,
                    }));
                }
                return None;
            }
            // A math function without substitution can be simplified and
            // reparsed now. This rejects an invalid winning declaration such
            // as `width: calc(foo)` during declaration parsing, rather than
            // letting it override a valid earlier declaration and fail only
            // during cascade resolution.
            //
            // Pure-number math (`calc(0)`, `calc(3 - 3)` — no dimension or
            // percentage anywhere, see
            // `math_source_has_dimension_or_percentage`) skips this
            // simplification: it would erase the calc type and let a
            // number-typed result satisfy `<length>` positions through the
            // unitless-zero rule (`flex: 1 2 calc(0)` must stay invalid
            // while `flex: calc(-1) calc(-1) 0` stays valid). Such values go
            // straight to the type-aware dummy check below.
            // `box-shadow` accepts `<length>`, not `<length-percentage>`,
            // for its four length slots. A mixed percentage expression cannot
            // be validated by replacing the whole math function with `1px`.
            if normalized_name == "box-shadow" && math_source_has_percentage(value.as_ref()) {
                return None;
            }
            let has_dimension = math_source_has_dimension_or_percentage(value.as_ref());
            if has_dimension
                && let Some(simplified) = crate::cascade::simplify_math_functions(value.as_ref())
            {
                let mut reparsed_input = ParserInput::new(simplified.as_ref());
                let mut reparsed = Parser::new(&mut reparsed_input);
                if let Some(parsed) = parse_value(name, &mut reparsed)
                    && reparsed.expect_exhausted().is_ok()
                {
                    return Some(parsed);
                }
                // Simplification succeeded but reparsed value is still not a
                // plain valid value (e.g. `calc(2em + 3ex)` for width). If the
                // math syntax itself is valid and the overall value structure is valid
                // for the property (e.g. `margin-top: calc(...)` single value vs
                // `margin-top: calc(...) auto` two values), treat as deferred.
                // Tab-size never allows percentages, even inside calc (Percentages: N/A).
                // Border-spacing likewise (CSS Tables 3 §6.1 Percentages: N/A).
                if (normalized_name == "tab-size" || normalized_name == "border-spacing")
                    && value.contains('%')
                {
                    return None;
                }
                if math_function_syntax_is_valid(value.as_ref())
                    && deferred_dummy_is_valid_for_property(value.as_ref(), name)
                {
                    return Some(PropertyValue::Deferred(DeferredValue {
                        property: name.to_ascii_lowercase().into(),
                        value,
                        key: key?,
                    }));
                } else {
                    return None;
                }
            }
            // If simplification itself fails (oversized), check if math syntax is valid and structurally valid before deferring.
            // Tab-size never allows percentages, even inside calc (Percentages: N/A).
            // Border-spacing likewise (CSS Tables 3 §6.1 Percentages: N/A).
            if (normalized_name == "tab-size" || normalized_name == "border-spacing")
                && value.contains('%')
            {
                return None;
            }
            if math_function_syntax_is_valid(value.as_ref())
                && deferred_dummy_is_valid_for_property(value.as_ref(), name)
            {
                return Some(PropertyValue::Deferred(DeferredValue {
                    property: name.to_ascii_lowercase().into(),
                    value,
                    key: key?,
                }));
            } else {
                return None;
            }
        }
        return Some(PropertyValue::Deferred(DeferredValue {
            property: name.to_ascii_lowercase().into(),
            value,
            key: key?,
        }));
    }
    input.reset(&start);

    // Dispatch property names using ASCII-lowercase comparisons.
    match normalized_name.as_str() {
        "all" => {
            input.expect_ident_matching("revert-layer").ok()?;
            Some(PropertyValue::AllRevertLayer)
        }
        "color" => parse_element_color(input, PropertyKey::Color),
        // CSS Backgrounds 3 §2.2 <https://www.w3.org/TR/css-backgrounds-3/#background-color>
        // "Base Color: the background-color property". Its value grammar is
        // `<color>`; preserve currentcolor as in the `color` arm above.
        "background-color" => parse_element_color(input, PropertyKey::BackgroundColor),
        // Wrapping in Arc reduces cascade memory (following similar DoS fixes,
        // but for performance, not security). `parse_font_family` cannot return
        // an empty Vec: `<family-name>#` requires at least one item, enforced by
        // its `if families.is_empty() { None }` check. Unlike counter-*, content,
        // and string-set, it needs no shared-empty-slot branch.
        "font-family" => parse_font_family(input).map(|v| PropertyValue::FontFamily(Arc::new(v))),
        "font-size" => parse_font_size(input),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        // CSS Inline 3 §5.1 line-height.
        // Accept `normal`, `<number [0,∞]>`, and `<length-percentage [0,∞]>`;
        // reject negatives and other keywords as invalid under the grammar.
        "line-height" => parse_line_height(input).map(PropertyValue::LineHeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        "list-style-type" => parse_list_style_type(input).map(PropertyValue::ListStyleType),
        "list-style-position" => {
            parse_list_style_position(input).map(PropertyValue::ListStylePosition)
        }
        "list-style-image" => parse_list_style_image(input).map(PropertyValue::ListStyleImage),
        // CSS Lists 3 §4 counter properties.
        // Spec defaults: reset = 0, increment = 1, set = 0.
        // Arc wrapping reduces cascade memory for DoS resistance (shallow
        // per-element clones). Empty lists use the shared Arc slot common to all
        // three properties (`empty_counter_entries`) to avoid per-node allocation
        // regressions, following the Content/StringSet pattern.
        "counter-reset" => {
            if input
                .try_parse(|i| i.expect_ident_matching("inherit"))
                .is_ok()
            {
                Some(PropertyValue::CounterResetInherit)
            } else {
                parse_counter_property(input, 0).map(|v| {
                    if v.is_empty() {
                        PropertyValue::CounterReset(empty_counter_entries())
                    } else {
                        PropertyValue::CounterReset(Arc::new(v))
                    }
                })
            }
        }
        "counter-increment" => parse_counter_property(input, 1).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterIncrement(empty_counter_entries())
            } else {
                PropertyValue::CounterIncrement(Arc::new(v))
            }
        }),
        "counter-set" => parse_counter_property(input, 0).map(|v| {
            if v.is_empty() {
                PropertyValue::CounterSet(empty_counter_entries())
            } else {
                PropertyValue::CounterSet(Arc::new(v))
            }
        }),
        // CSS Content 3 §1 content property.
        // Arc wrapping reduces cascade memory for DoS resistance (shallow
        // per-element clones). Empty lists use a shared Arc slot to avoid
        // per-node allocation regressions.
        "content" => parse_content(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Content(empty_content_list())
            } else {
                PropertyValue::Content(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.1.1 string-set.
        // Arc wrapping follows the rationale of similar DoS fixes.
        "string-set" => parse_string_set(input).map(|v| {
            if v.is_empty() {
                PropertyValue::StringSet(empty_string_set_entries())
            } else {
                PropertyValue::StringSet(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.2.1 running() position and CSS Positioned Layout 3 §3 sticky.
        // This scope accepts `static`, `sticky`, and `running(<custom-ident>)`;
        // `relative`, `absolute`, and `fixed` remain unimplemented and silently dropped.
        "position" => parse_position(input).map(PropertyValue::Position),
        "top" => parse_inset(input).map(PropertyValue::Top),
        "right" => parse_inset(input).map(PropertyValue::Right),
        "bottom" => parse_inset(input).map(PropertyValue::Bottom),
        "left" => parse_inset(input).map(PropertyValue::Left),
        // CSS Text 3 §6.1 text-align.
        // The spec defines a shorthand (text-align-all + text-align-last), but
        // we store one field (case (b) unsupported; see the `TextAlign` doc comment).
        "text-align" => parse_text_align(input).map(PropertyValue::TextAlign),
        // CSS Text 3 §8.2.1: retain the complete inherited, unordered keyword set.
        // The computed value serializes in first, end-mode, last order.
        "hanging-punctuation" => {
            parse_hanging_punctuation(input).map(PropertyValue::HangingPunctuation)
        }
        // CSS Text 4 `text-spacing` shorthand: preserve and expand both
        // longhand values; no spacing behavior is added here.
        "text-spacing" => {
            parse_text_spacing_shorthand(input).map(PropertyValue::TextSpacingShorthand)
        }
        // CSS Text 4 text-autospace. The computed value is an inherited
        // keyword/flag set; layout support consumes the `normal` and
        // `no-autospace` forms first, while the full grammar is preserved.
        "text-autospace" => parse_text_autospace(input).map(PropertyValue::TextAutospace),
        // CSS Text 4 `word-space-transform`; retain the specified keyword set only.
        "word-space-transform" => {
            parse_word_space_transform(input).map(PropertyValue::WordSpaceTransform)
        }
        // CSS Text 3 §8.1 text-indent — `<length-percentage>` component only
        // (`hanging`/`each-line` out of scope, `PropertyValue::TextIndent` doc).
        "text-indent" => parse_text_indent(input).map(PropertyValue::TextIndent),
        // CSS Box 3 §4.1 padding physical longhand.
        // Grammar: <length-percentage `[0,∞]`>. parse_padding_side enforces the
        // non-negative constraint (invalid values are dropped during parsing).
        // `auto` is not in the grammar and falls through the Dimension / Percentage
        // arms of parse_length_value, so it is rejected naturally.
        "padding-top" => parse_padding_side(input).map(PropertyValue::PaddingTop),
        "padding-right" => parse_padding_side(input).map(PropertyValue::PaddingRight),
        "padding-bottom" => parse_padding_side(input).map(PropertyValue::PaddingBottom),
        "padding-left" => parse_padding_side(input).map(PropertyValue::PaddingLeft),
        // CSS Box 3 §4.2 padding shorthand: `<'padding-top'>{1,4}`
        // <https://www.w3.org/TR/css-box-3/#padding-shorthand>.
        // parse_padding_shorthand applies the spec's 1–4 value expansion verbatim.
        "padding" => parse_padding_shorthand(input).map(PropertyValue::Padding),
        // CSS Logical Properties and Values 1 §4.4 padding-inline-start/-end /
        // padding-block-start/-end — physically fixed-mapped onto the
        // matching padding physical longhand (`PropertyValue::PaddingInline`
        // doc's Non-goal section covers the writing-mode/direction
        // rationale). Grammar/non-negative constraint is identical to the
        // physical longhand, so this reuses `parse_padding_side` verbatim.
        "padding-inline-start" => parse_padding_side(input).map(PropertyValue::PaddingLeft),
        "padding-inline-end" => parse_padding_side(input).map(PropertyValue::PaddingRight),
        "padding-block-start" => parse_padding_side(input).map(PropertyValue::PaddingTop),
        "padding-block-end" => parse_padding_side(input).map(PropertyValue::PaddingBottom),
        // CSS Logical Properties and Values 1 §4.4 padding-inline / padding-block
        // shorthand: `<'padding-top'>{1,2}`
        // <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline>.
        // 1-2 value expansion is identical for both (shared
        // `parse_padding_logical_shorthand` helper) — the physical axis the
        // resulting `StartEnd` maps onto is decided by which
        // `PropertyValue` variant wraps it, not by the parser.
        "padding-inline" => {
            parse_padding_logical_shorthand(input).map(PropertyValue::PaddingInline)
        }
        "padding-block" => parse_padding_logical_shorthand(input).map(PropertyValue::PaddingBlock),
        // CSS Box 3 §3.1 margin-* physical longhand.
        // Grammar: <length-percentage> | auto. Negatives are allowed by the spec;
        // layout interprets their meaning.
        "margin-top" => parse_margin_side(input).map(PropertyValue::MarginTop),
        "margin-right" => parse_margin_side(input).map(PropertyValue::MarginRight),
        "margin-bottom" => parse_margin_side(input).map(PropertyValue::MarginBottom),
        "margin-left" => parse_margin_side(input).map(PropertyValue::MarginLeft),
        // CSS Box 3 §3.2 margin shorthand. Expands 1–4 values. During cascade,
        // `PropertyValue::Margin` expands into four longhands in `parse_declaration_block`
        // and is therefore not normally observed (see the `PropertyValue::Margin`
        // doc and `crate::rule::expand_shorthand_into`).
        "margin" => parse_margin_shorthand(input).map(PropertyValue::Margin),
        // CSS Logical Properties and Values 1 §4.2 margin-inline-start/-end /
        // margin-block-start/-end — same physically fixed-mapped pattern as
        // padding-inline-*/padding-block-* above, reusing `parse_margin_side`
        // verbatim (grammar `<'margin-top'>` = `<length-percentage> | auto`
        // is identical to the physical longhand).
        "margin-inline-start" => parse_margin_side(input).map(PropertyValue::MarginLeft),
        "margin-inline-end" => parse_margin_side(input).map(PropertyValue::MarginRight),
        "margin-block-start" => parse_margin_side(input).map(PropertyValue::MarginTop),
        "margin-block-end" => parse_margin_side(input).map(PropertyValue::MarginBottom),
        // CSS Logical Properties and Values 1 §4.2 margin-inline / margin-block
        // shorthand: `<'margin-top'>{1,2}`
        // <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>. Same
        // shared-helper shape as padding-inline/padding-block above.
        "margin-inline" => parse_margin_logical_shorthand(input).map(PropertyValue::MarginInline),
        "margin-block" => parse_margin_logical_shorthand(input).map(PropertyValue::MarginBlock),
        // CSS Backgrounds 3 §3.3 border-width physical longhand. Grammar:
        // `<line-width>` = `<length [0,∞]> |
        // thin | medium | thick`. Unlike padding, it excludes `<percentage>`.
        // Specified keyword mappings: thin=1px, medium=3px, thick=5px.
        // Negative values violate the grammar and are dropped by
        // `parse_border_width_side`.
        "border-top-width" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderTopWidthCssWide(kw))
            } else {
                parse_border_width_side(input).map(PropertyValue::BorderTopWidth)
            }
        }
        "border-right-width" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderRightWidthCssWide(kw))
            } else {
                parse_border_width_side(input).map(PropertyValue::BorderRightWidth)
            }
        }
        "border-bottom-width" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderBottomWidthCssWide(kw))
            } else {
                parse_border_width_side(input).map(PropertyValue::BorderBottomWidth)
            }
        }
        "border-left-width" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderLeftWidthCssWide(kw))
            } else {
                parse_border_width_side(input).map(PropertyValue::BorderLeftWidth)
            }
        }
        // CSS Backgrounds 3 §3.2 border-style physical longhand. Grammar:
        // `<line-style>` has ten alternatives
        // (none / hidden / dotted / dashed / solid / double / groove / ridge /
        // inset / outset). Other keywords are silently dropped.
        "border-top-style" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderTopStyleCssWide(kw))
            } else {
                parse_border_style_side(input).map(PropertyValue::BorderTopStyle)
            }
        }
        "border-right-style" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderRightStyleCssWide(kw))
            } else {
                parse_border_style_side(input).map(PropertyValue::BorderRightStyle)
            }
        }
        "border-bottom-style" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderBottomStyleCssWide(kw))
            } else {
                parse_border_style_side(input).map(PropertyValue::BorderBottomStyle)
            }
        }
        "border-left-style" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderLeftStyleCssWide(kw))
            } else {
                parse_border_style_side(input).map(PropertyValue::BorderLeftStyle)
            }
        }
        // CSS Backgrounds 3 §3.1 border-color physical longhand, via
        // `parse_border_color`. The grammar also accepts `currentcolor` ahead
        // of `<color>` (CSS Color 3 §4.4). `BorderColor` preserves the specified
        // value distinction; used-value resolution belongs to painting.
        "border-top-color" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderTopColorCssWide(kw))
            } else {
                parse_border_color(input).map(PropertyValue::BorderTopColor)
            }
        }
        "border-right-color" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderRightColorCssWide(kw))
            } else {
                parse_border_color(input).map(PropertyValue::BorderRightColor)
            }
        }
        "border-bottom-color" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderBottomColorCssWide(kw))
            } else {
                parse_border_color(input).map(PropertyValue::BorderBottomColor)
            }
        }
        "border-left-color" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderLeftColorCssWide(kw))
            } else {
                parse_border_color(input).map(PropertyValue::BorderLeftColor)
            }
        }
        // CSS Backgrounds 3 §3.4 border shorthand: `<line-width> || <line-style>
        // || <color>` (any order, each component at most once, at least one present).
        // Apply the same Border to all four sides. During cascade,
        // `PropertyValue::Border` expands into 12 longhands (four sides × three
        // subproperties) in `parse_declaration_block` and is not normally observed
        // (see the `PropertyValue::Border` doc and `crate::rule::expand_shorthand_into`).
        "border" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderCssWide(kw))
            } else {
                parse_border_shorthand(input).map(PropertyValue::Border)
            }
        }
        "border-top" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderTopCssWide(kw))
            } else {
                parse_border_top_shorthand(input).map(PropertyValue::BorderTop)
            }
        }
        "border-right" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderRightCssWide(kw))
            } else {
                parse_border_right_shorthand(input).map(PropertyValue::BorderRight)
            }
        }
        "border-bottom" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderBottomCssWide(kw))
            } else {
                parse_border_bottom_shorthand(input).map(PropertyValue::BorderBottom)
            }
        }
        "border-left" => {
            if let Ok(kw) = input.try_parse(parse_css_wide_keyword_res) {
                Some(PropertyValue::BorderLeftCssWide(kw))
            } else {
                parse_border_left_shorthand(input).map(PropertyValue::BorderLeft)
            }
        }
        "border-style" => parse_border_style_shorthand(input).map(PropertyValue::BorderStyle),
        "border-width" => parse_border_width_shorthand(input).map(PropertyValue::BorderWidth),
        "border-color" => parse_border_color_shorthand(input).map(PropertyValue::BorderColor),
        // CSS Sizing 3 §3.1.1 preferred size property.
        // Of `auto | <length-percentage [0,∞]> | min-content | max-content
        // | fit-content(<length-percentage>)`, accept only `auto` and non-negative
        // `<length-percentage>`. Silently drop the unimplemented min-content,
        // max-content, and fit-content() forms; parse_width rejects negative
        // values under the spec's `[0,∞]` constraint.
        "width" => parse_width(input).map(PropertyValue::Width),
        // CSS Sizing 3 §3.1.1 preferred size — height.
        "height" => parse_height(input).map(PropertyValue::Height),
        // CSS Logical Properties 1 §4.1 inline-size / block-size share the
        // `width` / `height` grammar but stay logical until the cascade knows
        // the element's `writing-mode`.
        "inline-size" => parse_width(input).map(PropertyValue::InlineSize),
        "block-size" => parse_height(input).map(PropertyValue::BlockSize),
        // CSS Sizing 3 §3.2 max-size properties.
        // `none | <length-percentage [0,∞]> | min-content | max-content | fit-content`
        "max-width" => parse_max_size(input).map(PropertyValue::MaxWidth),
        "max-height" => parse_max_size(input).map(PropertyValue::MaxHeight),
        "min-width" => parse_min_size(input).map(PropertyValue::MinWidth),
        "min-height" => parse_min_size(input).map(PropertyValue::MinHeight),
        "min-block-size" => parse_min_size(input).map(PropertyValue::MinBlockSize),
        // CSS Sizing 3 §3.3 box-sizing.
        // Value grammar `content-box | border-box`, initial `content-box`,
        // not inherited, computed value = specified keyword.
        "box-sizing" => parse_box_sizing(input).map(PropertyValue::BoxSizing),
        // CSS Writing Modes 4 §2.1 direction.
        // Value grammar `ltr | rtl`, initial `ltr`, inherited;
        // computed value = specified keyword (see `Direction` doc).
        "direction" => parse_direction(input).map(PropertyValue::Direction),
        // CSS Overflow 3 §3.1 overflow-x/overflow-y physical longhand.
        // grammar: visible | hidden | clip | scroll |
        // auto, initial visible, not inherited. cross-axis computed-value
        // coupling is applied in phase 3 (`resolve_overflow`), not here —
        // this only carries the specified keyword.
        "overflow-x" => parse_overflow_value(input).map(PropertyValue::OverflowX),
        "overflow-y" => parse_overflow_value(input).map(PropertyValue::OverflowY),
        // CSS Overflow 3 §3.1 overflow shorthand: `<'overflow-block'>{1,2}`.
        // 1-2 value expansion via
        // parse_overflow_shorthand (mapped to physical x/y — `OverflowValue`
        // doc's Non-goal note).
        "overflow" => parse_overflow_shorthand(input).map(PropertyValue::Overflow),
        // CSS Text Decoration 4 §2.1 text-decoration-line grammar:
        // `none | [ underline || overline || line-through || blink ] |
        // spelling-error | grammar-error` (Level 4 adds two top-level
        // alternatives to the four Level 3 keywords; see `TextDecorationLine` doc).
        "text-decoration-line" => {
            parse_text_decoration_line(input).map(PropertyValue::TextDecorationLine)
        }
        // §2.2 text-decoration-style grammar: `solid | double | dotted |
        // dashed | wavy`.
        "text-decoration-style" => {
            parse_text_decoration_style(input).map(PropertyValue::TextDecorationStyle)
        }
        // §2.3 text-decoration-color grammar: `<color>`.
        "text-decoration-color" => {
            parse_text_decoration_color(input).map(PropertyValue::TextDecorationColor)
        }
        // §2.4 text-decoration shorthand: `<'text-decoration-line'> ||
        // <'text-decoration-style'> || <'text-decoration-color'> ||
        // <'text-decoration-thickness'>` (the ED adds thickness;
        // see `TextDecorationShorthand` doc).
        "text-decoration" => {
            parse_text_decoration_shorthand(input).map(PropertyValue::TextDecoration)
        }
        // CSS Text Decoration 4 ED §2.10.4 text-decoration-skip-ink.
        // Grammar: `auto | none | all` (see `TextDecorationSkipInk` doc).
        "text-decoration-skip-ink" => {
            parse_text_decoration_skip_ink(input).map(PropertyValue::TextDecorationSkipInk)
        }
        // ED §2.10.3 text-decoration-skip-spaces. Grammar:
        // `none | all | [ start || end ]` (see
        // `TextDecorationSkipSpaces` doc).
        "text-decoration-skip-spaces" => {
            parse_text_decoration_skip_spaces(input).map(PropertyValue::TextDecorationSkipSpaces)
        }
        // ED §2.4.1 text-decoration-thickness. Grammar:
        // `auto | from-font | <length-percentage>` (`<line-width>` is out of
        // scope; see `TextDecorationThickness` doc).
        "text-decoration-thickness" => {
            if input
                .try_parse(|i| i.expect_ident_matching("inherit"))
                .is_ok()
            {
                Some(PropertyValue::TextDecorationThicknessInherit)
            } else {
                parse_text_decoration_thickness(input).map(PropertyValue::TextDecorationThickness)
            }
        }
        // ED §2.9.1 text-decoration-inset. Grammar: `<length>{1,2} | auto`
        // (`<percentage>` is out of scope because WPT rejects it;
        // see `TextDecorationInset` doc).
        "text-decoration-inset" => {
            parse_text_decoration_inset(input).map(PropertyValue::TextDecorationInset)
        }
        // ED §3.4 text-emphasis-position. Grammar:
        // `auto | ([ over | under ] && [ right | left ]?)`
        // (see `TextEmphasisPosition` doc).
        "text-emphasis-position" => {
            parse_text_emphasis_position(input).map(PropertyValue::TextEmphasisPosition)
        }
        // ED §2.7 text-underline-position. Grammar:
        // `auto | [ from-font | under ] || [ left | right ]`
        // (see `TextUnderlinePosition` doc).
        "text-underline-position" => {
            parse_text_underline_position(input).map(PropertyValue::TextUnderlinePosition)
        }
        // CSS Text Decoration 4 text-emphasis-style. Preserve the tested
        // fill, shape, and string forms as inherited computed data.
        "text-emphasis-style" => {
            parse_text_emphasis_style(input).map(PropertyValue::TextEmphasisStyle)
        }
        "text-emphasis-color" => {
            parse_text_decoration_color(input).map(PropertyValue::TextEmphasisColor)
        }
        "text-emphasis" => parse_text_emphasis_shorthand(input).map(PropertyValue::TextEmphasis),
        // CSS Text Decoration 4 §2.8 text-underline-offset:
        // `auto | <length-percentage>`.
        "text-underline-offset" => {
            parse_text_underline_offset(input).map(PropertyValue::TextUnderlineOffset)
        }
        // CSS Paged Media 3 §8.1 page. Grammar: `auto | <custom-ident>`
        // (see `PageValue` doc).
        "page" => parse_page_value(input).map(PropertyValue::Page),
        // CSS 2.1 §10.8.1 vertical-align, restricted to `baseline` / `sub` /
        // `super` / `middle` / `text-top` / `text-bottom` / `<length>`
        // (`VerticalAlign` doc's "Scope carving" section — `top` / `bottom`
        // / `<percentage>` remain out of scope). initial `baseline`, not
        // inherited.
        "vertical-align" => parse_vertical_align(input).map(PropertyValue::VerticalAlign),
        // CSS Fonts 4 §2.4 font-style. grammar: `normal | italic | left |
        // right | oblique <angle [-90deg,90deg]>?`, restricted here to
        // `normal` / `italic` / bare `oblique` (`FontStyle` doc's "Scope
        // carving" section — the `<angle>` argument to `oblique` and
        // `left`/`right` are spec-valid but unimplemented). initial
        // `normal`, inherited, computed value = specified keyword
        // (angle-bearing branch unreachable at this scope).
        "font-style" => parse_font_style(input).map(PropertyValue::FontStyle),
        // CSS Fonts 3 `font-kerning`: preserve its inherited keyword value;
        // shaping behavior is intentionally outside this property-data slice.
        "font-kerning" => parse_font_kerning(input).map(PropertyValue::FontKerning),
        // CSS Fonts 4 `font-optical-sizing`: preserve its inherited keyword;
        // optical-size selection and shaping remain outside this data path.
        "font-optical-sizing" => {
            parse_font_optical_sizing(input).map(PropertyValue::FontOpticalSizing)
        }
        // CSS Fonts 4 `font-variant-emoji`: preserve its inherited keyword;
        // presentation behavior is intentionally outside this data path.
        "font-variant-emoji" => {
            parse_font_variant_emoji(input).map(PropertyValue::FontVariantEmoji)
        }
        // CSS Fonts 4 `font-language-override`: preserve the computed string
        // and canonicalize its trailing spaces; no font selection is performed.
        "font-language-override" => {
            parse_font_language_override(input).map(PropertyValue::FontLanguageOverride)
        }
        // CSS Fonts 4 `font-variant-ligatures`: preserve the individual
        // computed keywords from the pinned case; shaping remains out of scope.
        "font-variant-ligatures" => {
            parse_font_variant_ligatures(input).map(PropertyValue::FontVariantLigatures)
        }
        // Preserve the tested font-synthesis keyword set as CSSOM data only.
        "font-synthesis" => parse_font_synthesis(input).map(PropertyValue::FontSynthesis),
        // Preserve the CSS Fonts 3 computed keyword without enabling glyph shaping.
        "font-variant-position" => {
            parse_font_variant_position(input).map(PropertyValue::FontVariantPosition)
        }
        "font-palette" => parse_font_palette(input).map(PropertyValue::FontPalette),
        "font-variant-numeric" => {
            parse_font_variant_numeric(input).map(PropertyValue::FontVariantNumeric)
        }
        "font-variant-east-asian" => {
            parse_font_variant_east_asian(input).map(PropertyValue::FontVariantEastAsian)
        }
        // CSS Fonts 4 axis coordinates stay data-only; preserve the specified sequence.
        "font-variation-settings" => {
            parse_font_variation_settings(input).map(PropertyValue::FontVariationSettings)
        }
        "font-feature-settings" => {
            parse_font_feature_settings(input).map(PropertyValue::FontFeatureSettings)
        }
        // CSS Text 4 `text-spacing-trim`: preserve the specified keyword as
        // its computed value; layout behavior is intentionally out of scope.
        "text-spacing-trim" => parse_text_spacing_trim(input).map(PropertyValue::TextSpacingTrim),
        // CSS Text 4 text-transform grammar: `none | [capitalize | uppercase |
        // lowercase] || full-width || full-size-kana | math-auto`. Case and
        // width combinations are preserved as computed values; `math-auto` is
        // a standalone keyword. Initial `none`, inherited, computed value =
        // specified keyword.
        "text-transform" => parse_text_transform(input).map(PropertyValue::TextTransform),
        // CSS Display 3 §4 visibility. grammar: `visible |
        // hidden | collapse`. initial `visible`, inherited, computed value =
        // specified keyword (`Visibility` doc's "Scope carving" section —
        // `collapse`'s formatting-context-specific space-saving effect is
        // unimplemented, the keyword itself is fully accepted).
        "visibility" => parse_visibility(input).map(PropertyValue::Visibility),
        // CSS2 §9.9.1 z-index. grammar: `auto | <integer>` (`inherit` — the
        // propdef's third alternative — is the CSS-wide keyword, unhandled
        // here per the "CSS-wide keyword (canonical)" section above).
        // initial `auto`, not inherited, computed value = specified value.
        "z-index" => parse_z_index(input).map(PropertyValue::ZIndex),
        // CSS Text 3 §5.1 word-break. grammar (this crate's scope):
        // `normal | keep-all | break-all` — the spec's 4th, deprecated
        // `break-word` keyword is not implemented (`WordBreak` doc's
        // "Scope carving" section). initial `normal`, inherited, computed
        // value = specified keyword.
        "word-break" => parse_word_break(input).map(PropertyValue::WordBreak),
        // CSS Text 3 §5.4 overflow-wrap, grammar: `normal | break-word |
        // anywhere`. `word-wrap` is the spec's mandated legacy name alias
        // for this same property (`OverflowWrap` doc's "legacy alias"
        // section) — both names parse to the same `PropertyValue` variant /
        // `PropertyKey`. initial `normal`, inherited, computed value =
        // specified keyword.
        "overflow-wrap" | "word-wrap" => {
            parse_overflow_wrap(input).map(PropertyValue::OverflowWrap)
        }
        // CSS Text 3 §7.2 "Tracking: the letter-spacing property"
        // <https://www.w3.org/TR/css-text-3/#letter-spacing-property>.
        // The pinned computed-value case covers normal, length, percentage,
        // and simple mixed calc forms. Negative lengths are permitted by CSS
        // Text's stated implementation-dependent limit.
        "letter-spacing" => parse_letter_spacing(input).map(PropertyValue::LetterSpacing),
        // CSS Text 4 §8.1 "Word Spacing: the word-spacing property"
        // <https://drafts.csswg.org/css-text-4/#propdef-word-spacing>.
        "word-spacing" => parse_word_spacing(input).map(PropertyValue::WordSpacing),
        // CSS Fragmentation Module Level 3 §3.1 break-before / break-after.
        // grammar (this crate's scope): `auto | avoid | avoid-page | page`
        // (`BreakBetween` doc's "Scope carving" section). initial `auto`,
        // not inherited, computed value = specified keyword.
        "break-before" => parse_break_between(input).map(PropertyValue::BreakBefore),
        "break-after" => parse_break_between(input).map(PropertyValue::BreakAfter),
        // CSS Fragmentation Module Level 3 §3.2 break-inside. grammar (this
        // crate's scope): `auto | avoid | avoid-page` (`BreakInside` doc's
        // "Scope carving" section — a smaller, disjoint set from
        // `break-before`/`break-after`). initial `auto`, not inherited,
        // computed value = specified keyword.
        "break-inside" => parse_break_inside(input).map(PropertyValue::BreakInside),
        // CSS Fragmentation Module Level 3 §3.4 "Page Break Aliases" —
        // CSS2.1 legacy shorthands for break-before / break-after, with a
        // non-identity value remap (`BreakBetween` doc's "legacy
        // shorthand" section: `always` -> `page`, `auto`/`avoid` identity).
        // Both dispatch to the same `PropertyValue`/`PropertyKey` as
        // break-before/break-after (one cascade winner, not two).
        "page-break-before" => {
            parse_legacy_page_break_between(input).map(PropertyValue::BreakBefore)
        }
        "page-break-after" => parse_legacy_page_break_between(input).map(PropertyValue::BreakAfter),
        // CSS Fragmentation Module Level 3 §3.4 — CSS2.1 legacy shorthand
        // for break-inside, identity value mapping (`BreakInside` doc's
        // "legacy shorthand" section: CSS2.1's own `page-break-inside`
        // grammar is just `auto | avoid`).
        "page-break-inside" => {
            parse_legacy_page_break_inside(input).map(PropertyValue::BreakInside)
        }
        // CSS2 §9.5.1 float. grammar: `left | right | none` (`inherit` —
        // the propdef's fourth alternative — is the CSS-wide keyword,
        // unhandled here per the "CSS-wide keyword (canonical)" section
        // above). initial `none`, not inherited, computed value =
        // specified value; §9.7's forced `display` recomputation is
        // applied separately at phase 3 (`resolve_display_for_float` doc).
        "float" => parse_float(input).map(PropertyValue::Float),
        // CSS2 §9.5.2 clear. grammar: `none | left | right | both`
        // (`inherit` unhandled, same reason as `float` above). initial
        // `none`, not inherited, computed value = specified value.
        "clear" => parse_clear(input).map(PropertyValue::Clear),
        // CSS Text 3 §3 white-space. grammar (this crate's scope): `normal |
        // pre | nowrap | pre-wrap | pre-line` — the spec's 6th keyword
        // `break-spaces` is not implemented (`WhiteSpace` doc's "Scope
        // carving" section). initial `normal`, inherited, computed value =
        // specified keyword.
        "white-space" => parse_white_space(input).map(PropertyValue::WhiteSpace),
        // CSS Text 4 `white-space-collapse`: all six specified keywords are
        // preserved through the computed-value path; text behavior is deferred.
        "white-space-collapse" => {
            parse_white_space_collapse(input).map(PropertyValue::WhiteSpaceCollapse)
        }
        // CSS Text 4 `text-wrap-mode` and `text-wrap-style` longhands plus
        // the `text-wrap` shorthand, which expands into both longhands.
        "text-wrap" => parse_text_wrap_shorthand(input).map(PropertyValue::TextWrapShorthand),
        "text-wrap-mode" => parse_text_wrap_mode(input).map(PropertyValue::TextWrap),
        "text-wrap-style" => parse_text_wrap_style(input).map(PropertyValue::TextWrapStyle),
        // CSS Flexible Box Layout Module Level 1 §5.1
        // <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>.
        "flex-direction" => parse_flex_direction(input).map(PropertyValue::FlexDirection),
        // CSS Flexible Box Layout Module Level 1 §5.2
        // <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>.
        "flex-wrap" => parse_flex_wrap(input).map(PropertyValue::FlexWrap),
        // CSS Flexible Box Layout Module Level 1 §7.2.1
        // <https://www.w3.org/TR/css-flexbox-1/#flex-grow-property>.
        "flex-grow" => parse_nonneg_finite_number(input).map(PropertyValue::FlexGrow),
        // CSS Flexible Box Layout Module Level 1 §7.2.2
        // <https://www.w3.org/TR/css-flexbox-1/#flex-shrink-property>.
        "flex-shrink" => parse_nonneg_finite_number(input).map(PropertyValue::FlexShrink),
        // CSS Flexible Box Layout Module Level 1 §7.2.3
        // <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>.
        "flex-basis" => parse_flex_basis(input).map(PropertyValue::FlexBasis),
        // CSS Flexible Box Layout Module Level 1 §7.1 "The flex Shorthand"
        // <https://www.w3.org/TR/css-flexbox-1/#flex-property>.
        "flex" => parse_flex_shorthand(input).map(PropertyValue::Flex),
        // CSS Flexible Box Layout Module Level 1 §5.3
        // <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>.
        "flex-flow" => parse_flex_flow(input).map(PropertyValue::FlexFlow),
        // CSS Flexible Box Layout Module Level 1 §4.2
        // <https://www.w3.org/TR/css-flexbox-1/#order-property>.
        "order" => parse_order(input).map(PropertyValue::Order),
        // CSS Box Alignment Module Level 3 §5.1
        // <https://www.w3.org/TR/css-align-3/#propdef-justify-content>.
        "justify-content" => parse_content_alignment(input).map(PropertyValue::JustifyContent),
        // CSS Box Alignment Module Level 3 §5.1
        // <https://www.w3.org/TR/css-align-3/#propdef-align-content>.
        "align-content" => parse_content_alignment(input).map(PropertyValue::AlignContent),
        // CSS Box Alignment Module Level 3 §7.2
        // <https://www.w3.org/TR/css-align-3/#propdef-align-items>.
        "align-items" => parse_self_alignment(input).map(PropertyValue::AlignItems),
        // CSS Box Alignment Module Level 3 §6.2
        // <https://www.w3.org/TR/css-align-3/#propdef-align-self>.
        "align-self" => parse_align_self(input).map(PropertyValue::AlignSelf),
        // CSS Box Alignment Module Level 3 §8.1
        // <https://www.w3.org/TR/css-align-3/#propdef-row-gap>.
        "row-gap" => parse_gap_value(input).map(PropertyValue::RowGap),
        "grid-row-gap" => parse_gap_value(input).map(PropertyValue::RowGap),
        // CSS Box Alignment Module Level 3 §8.1
        // <https://www.w3.org/TR/css-align-3/#propdef-column-gap>.
        "column-gap" => parse_gap_value(input).map(PropertyValue::ColumnGap),
        "grid-column-gap" => parse_gap_value(input).map(PropertyValue::ColumnGap),
        // CSS Box Alignment Module Level 3 §8.2 "Gap Shorthand: the gap
        // property"
        // <https://www.w3.org/TR/css-align-3/#propdef-gap>.
        "gap" => parse_gap_shorthand(input).map(PropertyValue::Gap),
        "grid-gap" => parse_gap_shorthand(input).map(PropertyValue::Gap),
        // CSS Multi-column Layout Module Level 1.
        "column-count" => parse_column_count(input).map(PropertyValue::ColumnCount),
        "column-fill" => parse_column_fill(input).map(PropertyValue::ColumnFill),
        "column-width" => parse_column_width(input).map(PropertyValue::ColumnWidth),
        "columns" => parse_columns_shorthand(input).map(PropertyValue::Columns),
        // CSS Box Alignment Module Level 3 §5.2
        // <https://www.w3.org/TR/css-align-3/#propdef-place-content>.
        "place-content" => parse_place_content_shorthand(input).map(PropertyValue::PlaceContent),
        // CSS Text 3 §5.3 hyphens. grammar: `none | manual | auto`, initial
        // `manual`, inherited, computed value = specified keyword. `auto`
        // is parse-accepted as its own keyword (`Hyphens` doc's "Downstream
        // handoff" section — this crate does not collapse it to `manual` at
        // parse time).
        "hyphens" => parse_hyphens(input).map(PropertyValue::Hyphens),
        // CSS Text 4 `hyphenate-character` accepts `auto` or a CSS string.
        "hyphenate-character" => {
            parse_hyphenate_character(input).map(PropertyValue::HyphenateCharacter)
        }
        // CSS Text 4: one to three non-negative integer or `auto` values;
        // the parser expands omitted components into the computed triple.
        "hyphenate-limit-chars" => {
            parse_hyphenate_limit_chars(input).map(PropertyValue::HyphenateLimitChars)
        }
        // CSS Text 3 §5.2 line-break. grammar: `auto | loose | normal | strict | anywhere`.
        "line-break" => parse_line_break(input).map(PropertyValue::LineBreak),
        // CSS Text 3 §6.2 text-justify. grammar: `auto | none | inter-word | inter-character`.
        "text-justify" => parse_text_justify(input).map(PropertyValue::TextJustify),
        // CSS Text 3 §6.1 text-align-all longhand.
        "text-align-all" => parse_text_align_all(input).map(PropertyValue::TextAlignAll),
        // CSS Text 3 §6.1 text-align-last longhand.
        "text-align-last" => parse_text_align_last(input).map(PropertyValue::TextAlignLast),
        // CSS Writing Modes 3 §9.1 text-combine-upright. grammar: `none | all`.
        "text-combine-upright" => {
            parse_text_combine_upright(input).map(PropertyValue::TextCombineUpright)
        }
        // CSS Writing Modes 3 §5.1 text-orientation. grammar: `mixed | upright | sideways`.
        "text-orientation" => parse_text_orientation(input).map(PropertyValue::TextOrientation),
        // CSS Writing Modes 3 §2.2 unicode-bidi. grammar: `normal | embed | isolate | bidi-override | isolate-override | plaintext`.
        "unicode-bidi" => parse_unicode_bidi(input).map(PropertyValue::UnicodeBidi),
        // CSS Tables 3 §4 table-layout. grammar: `auto | fixed`
        // <https://www.w3.org/TR/css-tables-3/#table-layout-property>
        // (initial `auto`, not inherited). ASCII case-insensitive matching
        // follows sibling `parse_float`; the caller's `expect_exhausted`
        // (`rule.rs::DeclParser`) drops any remaining tokens.
        "table-layout" => parse_table_layout(input).map(PropertyValue::TableLayout),
        // CSS Overflow 3 §5.1 text-overflow. grammar: `clip | ellipsis`
        // (initial `clip`, not inherited).
        "text-overflow" => parse_text_overflow(input).map(PropertyValue::TextOverflow),
        // CSS Tables 3 §6 border-collapse. grammar: `collapse | separate`
        // <https://www.w3.org/TR/css-tables-3/#border-collapse-property>
        // (initial `separate`, inherited). Uses the same matching rules
        // as the `table-layout` arm above.
        "border-collapse" => parse_border_collapse(input).map(PropertyValue::BorderCollapse),
        // CSS Tables 3 §6.1 border-spacing. grammar: `<length>{1,2}`
        // <https://www.w3.org/TR/css-tables-3/#border-spacing-property>
        // (initial `0`, inherited, Percentages: N/A, negative illegal).
        // Values containing `calc()` take the upstream deferred path to
        // `Deferred` (as with `width`); this arm handles only plain `<length>`.
        "border-spacing" => parse_border_spacing(input).map(PropertyValue::BorderSpacing),
        // CSS Tables 3 §7 caption-side. grammar: `top | bottom`
        // <https://www.w3.org/TR/css-tables-3/#caption-side-property>
        // (initial `top`, inherited). Uses the same matching rules
        // as the `table-layout` arm above.
        "caption-side" => parse_caption_side(input).map(PropertyValue::CaptionSide),
        // CSS Tables 3 §8 empty-cells. grammar: `show | hide`
        // <https://www.w3.org/TR/css-tables-3/#empty-cells-property>
        // (initial `show`, inherited). Uses the same matching rules
        // as the `table-layout` arm above.
        "empty-cells" => parse_empty_cells(input).map(PropertyValue::EmptyCells),
        // CSS Fonts 4 §2.1 font shorthand — 16 longhands (6 grammar components plus
        // 10 modeled reset-only subproperties) are expanded by
        // `crate::rule::expand_shorthand_into` (see its doc).
        "font" => parse_font_shorthand(input).map(PropertyValue::Font),
        // CSS Text Module Level 3 §4.2
        // <https://www.w3.org/TR/css-text-3/#tab-size-property>.
        "tab-size" => parse_tab_size(input).map(PropertyValue::TabSize),
        // CSS Fonts Module Level 3 §6.6 font-variant-caps. grammar: `normal
        // | small-caps | all-small-caps | petite-caps | all-petite-caps |
        // unicase | titling-caps` — all 7 keywords implemented
        // (see `FontVariantCaps` doc's "Meaning of the seven keywords" section). initial
        // `normal`, inherited, computed value = specified keyword. The
        // `font-variant` shorthand has no dispatch arm of its own
        // (`FontVariantCaps` doc's "Scope carving" section — it would need
        // to reset longhands this crate does not have).
        "font-variant-caps" => parse_font_variant_caps(input).map(PropertyValue::FontVariantCaps),
        // CSS Content Module Level 3 §2.4.1 (legacy CSS2 §12.3.1 grammar
        // subset: `none | [ <string> <string> ]+`, `auto` / `match-parent`
        // not implemented — see `PropertyValue::Quotes` doc). Arc wrap +
        // shared-empty-slot follows the counter-* DoS mitigation pattern.
        "quotes" => parse_quotes_property(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Quotes(empty_quotes_entries())
            } else {
                PropertyValue::Quotes(Arc::new(v))
            }
        }),
        // CSS Text Decoration Module Level 3 §4 text-shadow.
        // Arc wrapping reduces cascade memory for DoS resistance, following the
        // Content/StringSet/counter-* fixes. Empty lists (`none`) use the shared
        // Arc slot (`empty_text_shadow_list`) to avoid per-node allocation
        // regressions.
        "text-shadow" => parse_text_shadow(input).map(|v| {
            if v.is_empty() {
                PropertyValue::TextShadow(empty_text_shadow_list())
            } else {
                PropertyValue::TextShadow(Arc::new(v))
            }
        }),
        // CSS Backgrounds and Borders 3 §5: this task supports the four
        // circular `<length>` radii only; percentages and slash-separated
        // elliptical radii remain a follow-up.
        "border-radius" => {
            if input
                .try_parse(|i| i.expect_ident_matching("inherit"))
                .is_ok()
            {
                Some(PropertyValue::BorderRadiusInherit)
            } else {
                parse_border_radius(input).map(PropertyValue::BorderRadius)
            }
        }
        "border-top-left-radius" => {
            parse_non_negative_length_percentage(input).map(PropertyValue::BorderRadiusTopLeft)
        }
        "border-top-right-radius" => {
            parse_non_negative_length_percentage(input).map(PropertyValue::BorderRadiusTopRight)
        }
        "border-bottom-right-radius" => {
            parse_non_negative_length_percentage(input).map(PropertyValue::BorderRadiusBottomRight)
        }
        "border-bottom-left-radius" => {
            parse_non_negative_length_percentage(input).map(PropertyValue::BorderRadiusBottomLeft)
        }
        // CSS Backgrounds and Borders 3 §6.1: multiple comma-separated
        // shadows are stored as an Arc list. `inset` is intentionally outside
        // this task's downstream-compatible subset.
        "box-shadow" => parse_box_shadow(input).map(|v| {
            if v.is_empty() {
                PropertyValue::BoxShadow(empty_box_shadow_list())
            } else {
                PropertyValue::BoxShadow(Arc::new(v))
            }
        }),
        // CSS UI 3 §4.1: outline is an any-order width/style/color shorthand and
        // does not participate in box-model sizing.
        "outline" => parse_outline(input).map(PropertyValue::Outline),
        // CSS UI 3 §§4.2–4.4 outline longhands. `auto` is accepted only by the
        // outline-style parser; border-style remains unchanged.
        "outline-width" => parse_border_width_side(input).map(PropertyValue::OutlineWidth),
        "outline-style" => parse_outline_style_side(input).map(PropertyValue::OutlineStyle),
        "outline-color" => parse_outline_color(input).map(PropertyValue::OutlineColor),
        // CSS UI 3 §4.5 outline-offset — <length>, initial 0, non-inherited.
        // Negative values are accepted and offset from the border edge; `<percentage>` is outside the grammar.
        "outline-offset" => parse_length_allow_negative(input).map(PropertyValue::OutlineOffset),
        // CSS Grid Layout Module Level 1 §7.2. The common WPT shorthand
        // form is `<grid-template-rows> / <grid-template-columns>`.
        "grid" => parse_grid_shorthand(input).map(PropertyValue::Grid),
        "grid-area" => parse_grid_area_shorthand(input).map(PropertyValue::GridArea),
        // CSS Grid Layout Module Level 1 §7.2
        // <https://www.w3.org/TR/css-grid-1/#track-sizing>.
        "grid-template-columns" => {
            parse_grid_template_tracks(input).map(PropertyValue::GridTemplateColumns)
        }
        "grid-template-rows" => {
            parse_grid_template_tracks(input).map(PropertyValue::GridTemplateRows)
        }
        // CSS Grid Layout Module Level 1 §7.3
        // <https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>.
        "grid-template-areas" => {
            parse_grid_template_areas(input).map(PropertyValue::GridTemplateAreas)
        }
        // CSS Grid Layout Module Level 1 §7.6
        // <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>.
        "grid-auto-columns" => {
            parse_grid_auto_track_list(input).map(PropertyValue::GridAutoColumns)
        }
        "grid-auto-rows" => parse_grid_auto_track_list(input).map(PropertyValue::GridAutoRows),
        // CSS Grid Layout Module Level 1 §7.7
        // <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>.
        "grid-auto-flow" => parse_grid_auto_flow(input).map(PropertyValue::GridAutoFlow),
        // CSS Grid Layout Module Level 1 §8.3
        // <https://www.w3.org/TR/css-grid-1/#line-placement>.
        "grid-row-start" => parse_grid_line(input).map(PropertyValue::GridRowStart),
        "grid-row-end" => parse_grid_line(input).map(PropertyValue::GridRowEnd),
        "grid-column-start" => parse_grid_line(input).map(PropertyValue::GridColumnStart),
        "grid-column-end" => parse_grid_line(input).map(PropertyValue::GridColumnEnd),
        // CSS Grid Layout Module Level 1 §8.4
        // <https://www.w3.org/TR/css-grid-1/#placement-shorthands>.
        "grid-row" => parse_grid_line_shorthand(input).map(PropertyValue::GridRow),
        "grid-column" => parse_grid_line_shorthand(input).map(PropertyValue::GridColumn),
        // CSS Box Alignment Module Level 3 §7.1
        // <https://www.w3.org/TR/css-align-3/#propdef-justify-items>.
        "justify-items" => parse_self_alignment(input).map(PropertyValue::JustifyItems),
        // CSS Box Alignment Module Level 3 §6.1
        // <https://www.w3.org/TR/css-align-3/#propdef-justify-self>.
        "justify-self" => parse_justify_self(input).map(PropertyValue::JustifySelf),
        // CSS Box Alignment Module Level 3 §7.3
        // <https://www.w3.org/TR/css-align-3/#propdef-place-items>.
        "place-items" => parse_place_items_shorthand(input).map(PropertyValue::PlaceItems),
        // CSS Box Alignment Module Level 3 §6.3
        // <https://www.w3.org/TR/css-align-3/#propdef-place-self>.
        "place-self" => parse_place_self_shorthand(input).map(PropertyValue::PlaceSelf),
        // CSS Fragmentation Module Level 3 §3.3 "Breaks Between Lines:
        // orphans, widows" <https://www.w3.org/TR/css-break-3/#widows-orphans>
        // (supersedes CSS 2.1 §13.3.2's original definition of these same 2
        // properties, unchanged grammar). Grammar: `<integer>`, restricted
        // to positive integers (`parse_positive_integer` doc). initial `2`,
        // inherited, computed value = specified integer.
        "orphans" => parse_positive_integer(input).map(PropertyValue::Orphans),
        "widows" => parse_positive_integer(input).map(PropertyValue::Widows),
        // CSS Writing Modes 4 §3.2 writing-mode.
        // value grammar `horizontal-tb | vertical-rl | vertical-lr |
        // sideways-rl | sideways-lr`, initial `horizontal-tb`, inherited.
        // All 5 keywords parse successfully. The 4 non-horizontal keywords
        // are preserved for CSSOM; `resolve_writing_mode` later normalizes the
        // renderer-facing fallback, not the computed CSSOM value.
        "writing-mode" => parse_writing_mode(input).map(PropertyValue::WritingMode),
        "ruby-position" => parse_ruby_position(input).map(PropertyValue::RubyPosition),
        // CSS Backgrounds and Borders 3 §2.4-§2.9. `background-clip` /
        // `background-origin` share the `<visual-box>` keyword parser
        // (`VisualBox` doc — the two differ only in initial value, handled
        // in `specified.rs`/`computed.rs`, not here).
        "background-repeat" => parse_background_repeat(input).map(PropertyValue::BackgroundRepeat),
        "background-attachment" => {
            parse_background_attachment(input).map(PropertyValue::BackgroundAttachment)
        }
        "background-clip" => parse_visual_box(input).map(PropertyValue::BackgroundClip),
        "background-origin" => parse_visual_box(input).map(PropertyValue::BackgroundOrigin),
        "background-size" => parse_background_size(input).map(PropertyValue::BackgroundSize),
        "background-position" => parse_bg_position(input).map(PropertyValue::BackgroundPosition),
        // CSS Backgrounds and Borders 3 §2.3
        // <https://www.w3.org/TR/css-backgrounds-3/#the-background-image>.
        "background-image" => parse_background_image(input).map(PropertyValue::BackgroundImage),
        // CSS Backgrounds and Borders 3 §2.10
        // <https://www.w3.org/TR/css-backgrounds-3/#the-background>. Any-order
        // `||` fan-out of the 8 longhands above, single layer only
        // (`BackgroundShorthand` doc's Non-goal section).
        "background" => parse_background_shorthand(input).map(PropertyValue::Background),
        // CSS Images Module Level 3 §5.1
        // <https://www.w3.org/TR/css-images-3/#the-object-fit>.
        "object-fit" => parse_object_fit(input).map(PropertyValue::ObjectFit),
        // CSS Images Module Level 3 §5.2
        // <https://www.w3.org/TR/css-images-3/#the-object-position>. Value:
        // `<position>` (CSS Values 4 §8.3), not `<bg-position>` —
        // `parse_position_strict` rejects the 3-value edge-offset form
        // `background-position`'s `parse_bg_position` accepts
        // (`parse_position_branch3_strict` doc's "Why" section).
        "object-position" => parse_position_strict(input).map(PropertyValue::ObjectPosition),
        "transform-origin" => parse_transform_origin(input),
        // CSS Color 4 §3.3
        // <https://www.w3.org/TR/css-color-4/#transparency>. Value:
        // `<opacity-value> = <number> | <percentage>`. The parsed number is
        // stored verbatim, out-of-range included — see
        // `PropertyValue::Opacity` doc's "specified preserves, computed
        // clamps" note and `parse_opacity_value` doc.
        "opacity" => parse_opacity_value(input).map(PropertyValue::Opacity),
        // CSS Compositing and Blending Level 1 §3.4.2
        // <https://www.w3.org/TR/compositing-1/#isolation>. Grammar:
        // `auto | isolate`.
        "isolation" => parse_isolation(input).map(PropertyValue::Isolation),
        // CSS Compositing and Blending Level 1 §3.4.1
        // <https://www.w3.org/TR/compositing-1/#mix-blend-mode>. Grammar:
        // `<blend-mode>` — see `MixBlendMode` doc for the 16-keyword list.
        "mix-blend-mode" => parse_mix_blend_mode(input).map(PropertyValue::MixBlendMode),
        // CSS Masking Level 1 §7.1 <https://www.w3.org/TR/css-masking-1/#the-mask-image>.
        // Grammar (single-layer subset — see `MaskImage` doc's scope
        // carving note): `none | <image> | <mask-source>`.
        "mask-image" => parse_mask_image(input).map(PropertyValue::MaskImage),
        // CSS Masking Level 1 §5.1 <https://www.w3.org/TR/css-masking-1/#the-clip-path>.
        // Grammar (`<basic-shape>`-free subset — see `ClipPath` doc's
        // scope carving note): `<clip-source> | <geometry-box> | none`.
        "clip-path" => parse_clip_path(input).map(PropertyValue::ClipPath),
        // CSS Transforms Level 1 §4 <https://www.w3.org/TR/css-transforms-1/#transform-property>.
        // `none` empty list convention — see `PropertyValue::Transform` doc.
        "transform" => parse_transform(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Transform(empty_transform_list())
            } else {
                PropertyValue::Transform(Arc::new(v))
            }
        }),
        // CSS Filter Effects Level 1 §5 <https://www.w3.org/TR/filter-effects-1/#FilterProperty>.
        // Same empty-list-means-none convention as `transform` above.
        "filter" => parse_filter(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Filter(empty_filter_list())
            } else {
                PropertyValue::Filter(Arc::new(v))
            }
        }),
        _ => None,
    }
}

pub(crate) fn contains_function_in_source(input: &str, wanted: &str) -> bool {
    fn scan(parser: &mut Parser<'_, '_>, wanted: &str) -> bool {
        let mut found = false;
        loop {
            let token = match parser.next() {
                Ok(token) => token.clone(),
                Err(_) => break,
            };
            match token {
                Token::Function(name) => {
                    if name.eq_ignore_ascii_case(wanted) {
                        found = true;
                    }
                    if parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, wanted))
                        })
                        .unwrap_or(false)
                    {
                        found = true;
                    }
                }
                Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                    if parser
                        .parse_nested_block(|nested| {
                            Ok::<_, ParseError<'_, ()>>(scan(nested, wanted))
                        })
                        .unwrap_or(false)
                    {
                        found = true;
                    }
                }
                _ => {}
            }
        }
        found
    }

    let mut parser_input = ParserInput::new(input);
    let mut parser = Parser::new(&mut parser_input);
    scan(&mut parser, wanted)
}
