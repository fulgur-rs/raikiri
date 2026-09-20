use std::sync::Arc;

use cssparser::color::parse_named_color;
use cssparser::{BasicParseError, ParseError, Parser, ParserInput, Token};
use smol_str::SmolStr;

use crate::Atom;

use super::types::*;

/// Property name + Parser から `PropertyValue` を produce。
/// 認識できない name / invalid value は `None`。
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

    // ascii-lowercase 比較で property name を dispatch。
    match normalized_name.as_str() {
        "color" => parse_color(input).map(PropertyValue::Color),
        // CSS Backgrounds 3 §2.2 <https://www.w3.org/TR/css-backgrounds-3/#background-color>
        // "Base Color: the background-color property"。value grammar は `<color>`、
        // 直上 sibling `color` arm と同じ parse_color reuse pattern。
        "background-color" => parse_color(input).map(PropertyValue::BackgroundColor),
        // Arc wrap は cascade memory 削減 (同種の DoS 対策 fix の
        // pattern 踏襲、perf 目的で security 対策ではない)。`parse_font_family` は
        // grammar 上 empty Vec を返さない (`<family-name>#` は 1 要素以上必須、
        // 同関数の `if families.is_empty() { None }` 参照) ため、counter-* /
        // content / string-set と異なり shared-empty-slot 分岐は不要。
        "font-family" => parse_font_family(input).map(|v| PropertyValue::FontFamily(Arc::new(v))),
        "font-size" => parse_font_size(input),
        "font-weight" => parse_font_weight(input).map(PropertyValue::FontWeight),
        // CSS Inline 3 §5.1 line-height。
        // `normal` / `<number [0,∞]>` / `<length-percentage [0,∞]>` を受理、
        // 負値と其他 keyword は spec grammar 違反として drop。
        "line-height" => parse_line_height(input).map(PropertyValue::LineHeight),
        "display" => parse_display(input).map(PropertyValue::Display),
        "list-style-type" => parse_list_style_type(input).map(PropertyValue::ListStyleType),
        "list-style-position" => {
            parse_list_style_position(input).map(PropertyValue::ListStylePosition)
        }
        // CSS Lists 3 §4 counter properties。
        // spec default: reset = 0、increment = 1、set = 0。
        // Arc wrap は cascade memory DoS 対策 (per-element
        // clone を shallow bump 化)、空 list は 3 property 共通 shared Arc slot
        // (`empty_counter_entries`) に落として per-node allocation regression を
        // 避ける (Content/StringSet の precedent と同 pattern)。
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
        // CSS Content 3 §1 content property。
        // Arc wrap は cascade memory DoS 対策 (per-element clone を
        // shallow bump 化)、empty list は shared Arc slot に落として per-node allocation
        // regression を避ける。
        "content" => parse_content(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Content(empty_content_list())
            } else {
                PropertyValue::Content(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.1.1 string-set。
        // Arc wrap は同種の DoS 対策 fix と同 rationale。
        "string-set" => parse_string_set(input).map(|v| {
            if v.is_empty() {
                PropertyValue::StringSet(empty_string_set_entries())
            } else {
                PropertyValue::StringSet(Arc::new(v))
            }
        }),
        // CSS GCPM 3 §1.2.1 position: running() および CSS Positioned Layout Module Level 3 §3 sticky。
        // 現状 scope では `static` + `sticky` + `running(<custom-ident>)` を受理、
        // `relative` / `absolute` / `fixed` は未実装 (将来対応) につき silent drop。
        "position" => parse_position(input).map(PropertyValue::Position),
        "top" => parse_inset(input).map(PropertyValue::Top),
        "right" => parse_inset(input).map(PropertyValue::Right),
        "bottom" => parse_inset(input).map(PropertyValue::Bottom),
        "left" => parse_inset(input).map(PropertyValue::Left),
        // CSS Text 3 §6.1 text-align。
        // spec 上 shorthand (text-align-all + text-align-last) だが単一 field で保持
        // ((b) 非対応、`TextAlign` doc-comment 参照)。
        "text-align" => parse_text_align(input).map(PropertyValue::TextAlign),
        // CSS Text 3 §8.1 text-indent — `<length-percentage>` component only
        // (`hanging`/`each-line` out of scope, `PropertyValue::TextIndent` doc).
        "text-indent" => parse_text_indent(input).map(PropertyValue::TextIndent),
        // CSS Box 3 §4.1 padding physical longhand。
        // grammar: <length-percentage `[0,∞]`> — non-negative constraint は
        // parse_padding_side が enforce (parse-time drop、spec-invalid → None)。
        // `auto` keyword は spec grammar に含まれず parse_length_value の Dimension /
        // Percentage arm fall-through で自然 reject。
        "padding-top" => parse_padding_side(input).map(PropertyValue::PaddingTop),
        "padding-right" => parse_padding_side(input).map(PropertyValue::PaddingRight),
        "padding-bottom" => parse_padding_side(input).map(PropertyValue::PaddingBottom),
        "padding-left" => parse_padding_side(input).map(PropertyValue::PaddingLeft),
        // CSS Box 3 §4.2 padding shorthand: `<'padding-top'>{1,4}`
        // <https://www.w3.org/TR/css-box-3/#padding-shorthand>。
        // 1-4 value expansion は parse_padding_shorthand が spec verbatim で適用。
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
        // <length-percentage> | auto の grammar、negative 許容 (spec 準拠、layout
        // 側で負値の意味付け)。
        "margin-top" => parse_margin_side(input).map(PropertyValue::MarginTop),
        "margin-right" => parse_margin_side(input).map(PropertyValue::MarginRight),
        "margin-bottom" => parse_margin_side(input).map(PropertyValue::MarginBottom),
        "margin-left" => parse_margin_side(input).map(PropertyValue::MarginLeft),
        // CSS Box 3 §3.2 margin shorthand. 1-4 value
        // expansion。cascade 段では `PropertyValue::Margin` は `parse_declaration_block`
        // 内で 4 longhand に展開されるため通常観測しない (詳細は
        // `PropertyValue::Margin` doc + `crate::rule::expand_shorthand_into`)。
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
        // CSS Backgrounds 3 §3.3 border-width physical longhand。grammar:
        // `<line-width>` = `<length [0,∞]> |
        // thin | medium | thick`。`<percentage>` は含まれない (padding とは違う点)。
        // keyword mapping は spec 規定値:
        // thin=1px、medium=3px、thick=5px。負値は spec grammar 違反 → drop
        // (`parse_border_width_side` が enforce)。
        "border-top-width" => parse_border_width_side(input).map(PropertyValue::BorderTopWidth),
        "border-right-width" => parse_border_width_side(input).map(PropertyValue::BorderRightWidth),
        "border-bottom-width" => {
            parse_border_width_side(input).map(PropertyValue::BorderBottomWidth)
        }
        "border-left-width" => parse_border_width_side(input).map(PropertyValue::BorderLeftWidth),
        // CSS Backgrounds 3 §3.2 border-style physical longhand。grammar:
        // `<line-style>` = 10 alternative
        // (none / hidden / dotted / dashed / solid / double / groove / ridge /
        // inset / outset)。他 keyword は silent drop。
        "border-top-style" => parse_border_style_side(input).map(PropertyValue::BorderTopStyle),
        "border-right-style" => parse_border_style_side(input).map(PropertyValue::BorderRightStyle),
        "border-bottom-style" => {
            parse_border_style_side(input).map(PropertyValue::BorderBottomStyle)
        }
        "border-left-style" => parse_border_style_side(input).map(PropertyValue::BorderLeftStyle),
        // CSS Backgrounds 3 §3.1 border-color physical longhand
        // (`parse_border_color` 経由)。grammar: `<color>` に加え
        // `currentcolor` keyword を先取り (CSS Color 3 §4.4)。`BorderColor` enum
        // で specified value distinction を保持し、used-value resolution は
        // paint scope 責務。
        "border-top-color" => parse_border_color(input).map(PropertyValue::BorderTopColor),
        "border-right-color" => parse_border_color(input).map(PropertyValue::BorderRightColor),
        "border-bottom-color" => parse_border_color(input).map(PropertyValue::BorderBottomColor),
        "border-left-color" => parse_border_color(input).map(PropertyValue::BorderLeftColor),
        // CSS Backgrounds 3 §3.4 border shorthand: `<line-width> || <line-style>
        // || <color>` (any-order、each component at most once、at least 1 present)。
        // 4 side 全てに同一 Border を配る。cascade 段では
        // `PropertyValue::Border` は `parse_declaration_block` 内で 12 longhand
        // (4 side × 3 sub-property) に展開されるため通常観測しない (詳細は
        // `PropertyValue::Border` doc + `crate::rule::expand_shorthand_into`)。
        "border" => parse_border_shorthand(input).map(PropertyValue::Border),
        "border-style" => parse_border_style_shorthand(input).map(PropertyValue::BorderStyle),
        "border-width" => parse_border_width_shorthand(input).map(PropertyValue::BorderWidth),
        "border-color" => parse_border_color_shorthand(input).map(PropertyValue::BorderColor),
        // CSS Sizing 3 §3.1.1 preferred size property。
        // grammar: `auto | <length-percentage [0,∞]> | min-content | max-content
        // | fit-content(<length-percentage>)` のうち `auto` + non-negative
        // `<length-percentage>` のみ受理、min-content / max-content / fit-content()
        // は未実装 (将来対応) として silent drop、負値は spec `[0,∞]`
        // violation として drop (parse_width が enforce)。
        "width" | "inline-size" => parse_width(input).map(PropertyValue::Width),
        // CSS Sizing 3 §3.1.1 preferred size — height。
        "height" | "block-size" => parse_height(input).map(PropertyValue::Height),
        // CSS Sizing 3 §3.2 max-size properties.
        // `none | <length-percentage [0,∞]> | min-content | max-content | fit-content`
        "max-width" => parse_max_size(input).map(PropertyValue::MaxWidth),
        "max-height" => parse_max_size(input).map(PropertyValue::MaxHeight),
        "min-width" => parse_min_size(input).map(PropertyValue::MinWidth),
        "min-height" => parse_min_size(input).map(PropertyValue::MinHeight),
        // CSS Sizing 3 §3.3 box-sizing。
        // value grammar `content-box | border-box`、initial `content-box`、
        // not inherited、computed value = specified keyword。
        "box-sizing" => parse_box_sizing(input).map(PropertyValue::BoxSizing),
        // CSS Writing Modes 4 §2.1 direction。
        // value grammar `ltr | rtl`、initial `ltr`、inherited、
        // computed value = specified keyword (`Direction` doc 参照)。
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
        // spelling-error | grammar-error` (Level 3 の 4 keyword に Level 4
        // の top-level 2 alternative が追加、`TextDecorationLine` doc 参照)。
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
        // <'text-decoration-thickness'> (ED に thickness が追加、
        // `TextDecorationShorthand` doc 参照)。
        "text-decoration" => {
            parse_text_decoration_shorthand(input).map(PropertyValue::TextDecoration)
        }
        // CSS Text Decoration 4 ED §2.10.4 text-decoration-skip-ink.
        // grammar: `auto | none | all` (`TextDecorationSkipInk` doc 参照)。
        "text-decoration-skip-ink" => {
            parse_text_decoration_skip_ink(input).map(PropertyValue::TextDecorationSkipInk)
        }
        // ED §2.10.3 text-decoration-skip-spaces. grammar:
        // `none | all | [ start || end ]` (`TextDecorationSkipSpaces`
        // doc 参照)。
        "text-decoration-skip-spaces" => {
            parse_text_decoration_skip_spaces(input).map(PropertyValue::TextDecorationSkipSpaces)
        }
        // ED §2.4.1 text-decoration-thickness. grammar:
        // `auto | from-font | <length-percentage>` (`<line-width>` は scope
        // 外、`TextDecorationThickness` doc 参照)。
        "text-decoration-thickness" => {
            parse_text_decoration_thickness(input).map(PropertyValue::TextDecorationThickness)
        }
        // ED §2.9.1 text-decoration-inset. grammar: `<length>{1,2} | auto`
        // (`<percentage>` は WPT が reject するため scope 外、
        // `TextDecorationInset` doc 参照)。
        "text-decoration-inset" => {
            parse_text_decoration_inset(input).map(PropertyValue::TextDecorationInset)
        }
        // ED §3.4 text-emphasis-position. grammar:
        // `auto | ([ over | under ] && [ right | left ]?)`
        // (`TextEmphasisPosition` doc 参照)。
        "text-emphasis-position" => {
            parse_text_emphasis_position(input).map(PropertyValue::TextEmphasisPosition)
        }
        // ED §2.7 text-underline-position. grammar:
        // `auto | [ from-font | under ] || [ left | right ]`
        // (`TextUnderlinePosition` doc 参照)。
        "text-underline-position" => {
            parse_text_underline_position(input).map(PropertyValue::TextUnderlinePosition)
        }
        // CSS Paged Media 3 §8.1 page. grammar: `auto | <custom-ident>`
        // (`PageValue` doc 参照)。
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
        // CSS Text Module Level 3 §2.1 text-transform. grammar: `none |
        // [capitalize | uppercase | lowercase] || full-width ||
        // full-size-kana`, restricted here to `none` / `capitalize` /
        // `uppercase` / `lowercase` (`TextTransform` doc's "Scope carving"
        // section — `full-width` / `full-size-kana` are spec-valid but
        // unimplemented). initial `none`, inherited, computed value =
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
        // grammar: `normal | <length>`, initial `normal`, inherited,
        // percentage NOT supported ("Percentages: n/a"), negative lengths
        // allowed ("Values may be negative, but there may be
        // implementation-dependent limits.") — see `parse_letter_or_word_spacing`.
        "letter-spacing" => parse_letter_or_word_spacing(input).map(PropertyValue::LetterSpacing),
        // CSS Text 3 §7.1 "Word Spacing: the word-spacing property"
        // <https://www.w3.org/TR/css-text-3/#word-spacing-property>. Same
        // `normal | <length>` grammar as `letter-spacing` above.
        "word-spacing" => parse_letter_or_word_spacing(input).map(PropertyValue::WordSpacing),
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
        // CSS Text 4 §5 text-wrap (subset: single `wrap | nowrap` keyword;
        // full shorthand with wrap-style deferred — see the TextWrapMode doc).
        "text-wrap" => parse_text_wrap_mode(input).map(PropertyValue::TextWrap),
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
        // (initial `auto`, not inherited). ASCII case-insensitive
        // matching は sibling `parse_float` と同 flavor、余剰 token は
        // caller (`rule.rs::DeclParser`) の `expect_exhausted` が drop する。
        "table-layout" => parse_table_layout(input).map(PropertyValue::TableLayout),
        // CSS Tables 3 §6 border-collapse. grammar: `collapse | separate`
        // <https://www.w3.org/TR/css-tables-3/#border-collapse-property>
        // (initial `separate`, inherited). matching 規則は直上の
        // `table-layout` arm と同じ。
        "border-collapse" => parse_border_collapse(input).map(PropertyValue::BorderCollapse),
        // CSS Tables 3 §6.1 border-spacing. grammar: `<length>{1,2}`
        // <https://www.w3.org/TR/css-tables-3/#border-spacing-property>
        // (initial `0`, inherited, Percentages: N/A, negative illegal).
        // `calc()` 混じりは上流の deferred path が `Deferred` に回す
        // (`width` 等と同型) — ここは plain `<length>` のみ扱う。
        "border-spacing" => parse_border_spacing(input).map(PropertyValue::BorderSpacing),
        // CSS Tables 3 §7 caption-side. grammar: `top | bottom`
        // <https://www.w3.org/TR/css-tables-3/#caption-side-property>
        // (initial `top`, inherited). matching 規則は直上の
        // `table-layout` arm と同じ。
        "caption-side" => parse_caption_side(input).map(PropertyValue::CaptionSide),
        // CSS Tables 3 §8 empty-cells. grammar: `show | hide`
        // <https://www.w3.org/TR/css-tables-3/#empty-cells-property>
        // (initial `show`, inherited). matching 規則は直上の
        // `table-layout` arm と同じ。
        "empty-cells" => parse_empty_cells(input).map(PropertyValue::EmptyCells),
        // CSS Fonts 4 §2.1 font shorthand — 6 longhand への展開は
        // `crate::rule::expand_shorthand_into` が行う (同 doc 参照)。
        "font" => parse_font_shorthand(input).map(PropertyValue::Font),
        // CSS Text Module Level 3 §4.2
        // <https://www.w3.org/TR/css-text-3/#tab-size-property>.
        "tab-size" => parse_tab_size(input).map(PropertyValue::TabSize),
        // CSS Fonts Module Level 3 §6.6 font-variant-caps. grammar: `normal
        // | small-caps | all-small-caps | petite-caps | all-petite-caps |
        // unicase | titling-caps` — all 7 keywords implemented
        // (`FontVariantCaps` doc's "7 keyword の意味" section). initial
        // `normal`, inherited, computed value = specified keyword. The
        // `font-variant` shorthand has no dispatch arm of its own
        // (`FontVariantCaps` doc's "Scope carving" section — it would need
        // to reset longhands this crate does not have).
        "font-variant-caps" => parse_font_variant_caps(input).map(PropertyValue::FontVariantCaps),
        // CSS Content Module Level 3 §2.4.1 (legacy CSS2 §12.3.1 grammar
        // subset: `none | [ <string> <string> ]+`, `auto` / `match-parent`
        // not implemented — see `PropertyValue::Quotes` doc). Arc wrap +
        // shared-empty-slot は counter-* と同じ DoS 対策 pattern。
        "quotes" => parse_quotes_property(input).map(|v| {
            if v.is_empty() {
                PropertyValue::Quotes(empty_quotes_entries())
            } else {
                PropertyValue::Quotes(Arc::new(v))
            }
        }),
        // CSS Text Decoration Module Level 3 §4 text-shadow。
        // Arc wrap は cascade memory DoS 対策 (同種の Content/StringSet/
        // counter-* fix の pattern 踏襲)、空 list (`none`) は shared Arc slot
        // (`empty_text_shadow_list`) に落として per-node allocation
        // regression を避ける。
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
        // 負値も受理し、border edge からの offset を示す。`<percentage>` は grammar 外。
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
        // All 5 keywords parse successfully — the computed-value collapse of
        // the 4 non-horizontal keywords happens later, in
        // `resolve_writing_mode` (`WritingMode` doc's Non-goal section), not
        // here.
        "writing-mode" => parse_writing_mode(input).map(PropertyValue::WritingMode),
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

/// `<color>` を parse する。
///
/// cssparser 0.37 は (0.36 までと異なり) 汎用 `Color` enum / `Color::parse` を
/// 提供しない — それは別 crate `cssparser-color` 側に移った。ここでは
/// 各 form の parse を自前 (独立実装) で組み立て、hex / named / rgb() /
/// color() / lab() / lch() / oklab() / oklch() / color-mix() をカバーする:
///
/// - **Hex** (`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`) は
///   [`CssColor::from_hex`] を呼び出す — CSS Color 4 §5.2 準拠の 独立実装 実装。
///   `Token::Hash` / `Token::IDHash` の payload は leading `#` を含まないため
///   そのまま渡す。
/// - **Named color** は `parse_named_color` (140+ CSS Color L3 keyword table を
///   再実装しない方針のため cssparser の table を暫定利用)。
/// - **`rgb()` / `rgba()` function form** は [`parse_rgb_function`] で
///   `parse_nested_block` 経由の手動 parse。
///
/// `transparent` keyword は CSS Color 4 §6.3 "The transparent keyword"
/// <https://www.w3.org/TR/css-color-4/#transparent-color> で
/// `rgba(0, 0, 0, 0)` の shorthand と規定される — `parse_named_color` の
/// (r, g, b) は alpha を返さないため、Ident arm 手前で明示 branch して
/// [`CssColor::TRANSPARENT`] を返す。
///
/// `from` を先頭に置く CSS Color 5 の relative color syntax は、origin と
/// channel/math grammar を検証する。cascade context を持たないため、解決値は
/// origin color の bounded approximation を保持し、`var()` origin は deferred
/// value として後段へ渡す。
///
/// Modern color syntax の `none`（missing component）は構文上受理し、bounded
/// model では一時的に zero component として扱う。missing-component の
/// carry-forward と computed-value serialization は後段の未実装範囲である。
/// Lab/OKLab の lightness が black/white boundary にある場合は conversion 側で
/// a/b や chroma にかかわらず boundary color へ固定する。それ以外の
/// out-of-gamut は 8-bit sRGB への bounded approximation であり、CSS Color 4 の
/// 完全な gamut mapping は未対応である。`color-mix()` の interpolation では、
/// Lab-family の座標を sRGB へ先に clip せず、指定空間での計算後にだけ変換する。
pub(crate) fn parse_color(input: &mut Parser<'_, '_>) -> Option<CssColor> {
    parse_color_float(input, 0).map(ParsedColor::to_css_color)
}

/// CSS system-color keywords are context-dependent at computed-value time.
/// The parser has no document/UA color context, so preserve their syntactic
/// validity and use a deterministic black/white approximation for the bounded
/// `CssColor` model.
fn parse_system_color(name: &str) -> Option<CssColor> {
    let is_system = matches!(
        name.to_ascii_lowercase().as_str(),
        "activetext"
            | "buttonborder"
            | "buttonface"
            | "buttontext"
            | "canvas"
            | "canvastext"
            | "field"
            | "fieldtext"
            | "graytext"
            | "highlight"
            | "highlighttext"
            | "linktext"
            | "mark"
            | "marktext"
            | "visitedtext"
            | "selecteditem"
            | "selecteditemtext"
            | "accentcolor"
            | "accentcolortext"
    );
    if !is_system {
        return None;
    }
    let light = matches!(
        name.to_ascii_lowercase().as_str(),
        "buttonface" | "canvas" | "field" | "highlighttext" | "marktext" | "selecteditemtext"
    );
    Some(if light {
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    } else {
        CssColor::BLACK
    })
}

fn parse_alpha_color_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    input.expect_ident_matching("from")?;
    let origin = parse_relative_origin(input, color_mix_depth)?;
    let alpha = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        if input
            .try_parse(|i| i.expect_ident_matching("alpha"))
            .is_ok()
        {
            origin.alpha
        } else if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            0.0
        } else if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
            percentage.clamp(0.0, 1.0)
        } else if let Ok(number) = input.try_parse(|i| expect_number_stable(i)) {
            number.clamp(0.0, 1.0)
        } else {
            // The remaining valid form is a relative math expression such as
            // `calc(alpha * 0.5)`. Validate it with the alpha-only channel
            // set; evaluation is deferred to the context-aware cascade.
            parse_relative_component(input, RelativeColorKind::Alpha, 0)?;
            origin.alpha
        }
    } else {
        origin.alpha
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        origin.to_srgb(),
        alpha,
    ))
}

fn parse_light_dark_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let light =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let _dark =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    // Color-scheme selection is a computed-value concern. Use the light branch
    // in this context-free specified-value parser.
    Ok(light)
}

fn parse_contrast_color_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    let [r, g, b] = origin.to_srgb().map(|component| component.clamp(0.0, 1.0));
    // Relative luminance is sufficient for the parser's deterministic
    // two-candidate fallback. The contrast-color grammar itself is validated
    // independently of this approximation.
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    Ok(ParsedColor::from_css_color(if luminance > 0.5 {
        CssColor::BLACK
    } else {
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }))
}

fn is_color_layers_blend_mode(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "normal"
            | "multiply"
            | "screen"
            | "overlay"
            | "darken"
            | "lighten"
            | "color-dodge"
            | "color-burn"
            | "hard-light"
            | "soft-light"
            | "difference"
            | "exclusion"
            | "hue"
            | "saturation"
            | "color"
            | "luminosity"
    )
}

fn composite_color_layers(bottom: ParsedColor, top: ParsedColor) -> ParsedColor {
    let bottom_rgb = bottom.to_srgb();
    let top_rgb = top.to_srgb();
    let bottom_alpha = bottom.alpha.clamp(0.0, 1.0);
    let top_alpha = top.alpha.clamp(0.0, 1.0);
    let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
    if alpha == 0.0 {
        return ParsedColor::from_coordinates(ParsedColorSpace::Srgb, [0.0; 3], 0.0);
    }
    let rgb = std::array::from_fn(|index| {
        (top_rgb[index] * top_alpha + bottom_rgb[index] * bottom_alpha * (1.0 - top_alpha)) / alpha
    });
    ParsedColor::from_coordinates(ParsedColorSpace::Srgb, rgb, alpha)
}

fn parse_color_layers_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    // An optional blend mode is followed by a comma. Trying the complete
    // optional prefix atomically lets a color keyword such as `red` remain
    // the first layer when it is not a blend mode.
    let _has_blend_mode = input
        .try_parse(|i| {
            let name = i.expect_ident()?.clone();
            i.expect_comma()?;
            if is_color_layers_blend_mode(name.as_ref()) {
                Ok(())
            } else {
                Err(i.new_custom_error::<(), ()>(()))
            }
        })
        .is_ok();
    let mut result =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    while input.try_parse(|i| i.expect_comma()).is_ok() {
        let layer =
            parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
        result = composite_color_layers(result, layer);
    }
    Ok(result)
}

pub(crate) fn parse_color_float(
    input: &mut Parser<'_, '_>,
    color_mix_depth: usize,
) -> Option<ParsedColor> {
    let token = input.next().ok()?.clone();
    match token {
        Token::Hash(ref value) | Token::IDHash(ref value) => {
            CssColor::from_hex(value).map(ParsedColor::from_css_color)
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("transparent") => {
            Some(ParsedColor::from_css_color(CssColor::TRANSPARENT))
        }
        Token::Ident(ref name) if name.eq_ignore_ascii_case("currentcolor") => {
            Some(ParsedColor::from_css_color(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }))
        }
        Token::Ident(ref name) => {
            if let Some(color) = parse_system_color(name) {
                Some(ParsedColor::from_css_color(color))
            } else {
                let (r, g, b) = parse_named_color(name).ok()?;
                Some(ParsedColor::from_css_color(CssColor { r, g, b, a: 255 }))
            }
        }
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            input
                .parse_nested_block(|nested| {
                    parse_rgb_function_or_relative(nested, color_mix_depth)
                })
                .ok()
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("color") => input
            .parse_nested_block(|nested| parse_color_function_or_relative(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("lab") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Lab, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("lch") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Lch, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("oklab") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Oklab, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("oklch") => input
            .parse_nested_block(|i| {
                parse_lab_function_or_relative(i, LabFunction::Oklch, color_mix_depth)
            })
            .ok(),
        Token::Function(ref name)
            if name.eq_ignore_ascii_case("hsl") || name.eq_ignore_ascii_case("hsla") =>
        {
            input
                .parse_nested_block(|i| parse_hsl_function_or_relative(i, color_mix_depth))
                .ok()
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("hwb") => input
            .parse_nested_block(|i| parse_hwb_function_or_relative(i, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("alpha") => input
            .parse_nested_block(|nested| parse_alpha_color_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("light-dark") => input
            .parse_nested_block(|nested| parse_light_dark_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("contrast-color") => input
            .parse_nested_block(|nested| parse_contrast_color_function(nested, color_mix_depth))
            .ok(),
        Token::Function(ref name) if name.eq_ignore_ascii_case("color-layers") => {
            if color_mix_depth >= MAX_COLOR_MIX_NESTING_DEPTH {
                None
            } else {
                input
                    .parse_nested_block(|nested| {
                        parse_color_layers_function(nested, color_mix_depth.saturating_add(1))
                    })
                    .ok()
            }
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("color-mix") => {
            if color_mix_depth >= MAX_COLOR_MIX_NESTING_DEPTH {
                None
            } else {
                input
                    .parse_nested_block(|nested| {
                        parse_color_mix_function(nested, color_mix_depth.saturating_add(1))
                    })
                    .ok()
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LabFunction {
    Lab,
    Lch,
    Oklab,
    Oklch,
}

#[derive(Clone, Copy)]
enum RelativeColorKind {
    Rgb,
    Alpha,
    Hsl,
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    ColorRgb,
    ColorXyz,
}

fn parse_rgb_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Rgb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_rgb_function(input)
    }
}

fn parse_hsl_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Hsl, color_mix_depth)
    } else {
        input.reset(&start);
        parse_hsl_function(input)
    }
}

fn parse_hwb_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::Hwb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_hwb_function(input)
    }
}

fn parse_lab_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    function: LabFunction,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    let relative_kind = match function {
        LabFunction::Lab => RelativeColorKind::Lab,
        LabFunction::Lch => RelativeColorKind::Lch,
        LabFunction::Oklab => RelativeColorKind::Oklab,
        LabFunction::Oklch => RelativeColorKind::Oklch,
    };
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, relative_kind, color_mix_depth)
    } else {
        input.reset(&start);
        parse_lab_function(input, function)
    }
}

fn parse_color_function_or_relative<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let start = input.state();
    if input.try_parse(|i| i.expect_ident_matching("from")).is_ok() {
        parse_relative_color_after_from(input, RelativeColorKind::ColorRgb, color_mix_depth)
    } else {
        input.reset(&start);
        parse_color_function(input)
    }
}

fn relative_kind_for_color_space(name: &str) -> Option<RelativeColorKind> {
    match name.to_ascii_lowercase().as_str() {
        "srgb" | "srgb-linear" | "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020"
        | "prophoto-rgb" => Some(RelativeColorKind::ColorRgb),
        "xyz" | "xyz-d50" | "xyz-d65" => Some(RelativeColorKind::ColorXyz),
        _ => None,
    }
}

fn relative_ident_allowed(kind: RelativeColorKind, name: &str) -> bool {
    match kind {
        RelativeColorKind::Rgb | RelativeColorKind::ColorRgb => {
            matches!(name, "r" | "g" | "b" | "alpha")
        }
        RelativeColorKind::Alpha => name == "alpha",
        RelativeColorKind::Hsl => matches!(name, "h" | "s" | "l" | "alpha"),
        RelativeColorKind::Hwb => matches!(name, "h" | "w" | "b" | "alpha"),
        RelativeColorKind::Lab | RelativeColorKind::Oklab => {
            matches!(name, "l" | "a" | "b" | "alpha")
        }
        RelativeColorKind::Lch | RelativeColorKind::Oklch => {
            matches!(name, "l" | "c" | "h" | "alpha")
        }
        RelativeColorKind::ColorXyz => matches!(name, "x" | "y" | "z" | "alpha"),
    }
}

fn relative_component_is_hue(kind: RelativeColorKind, index: usize) -> bool {
    match kind {
        RelativeColorKind::Hsl | RelativeColorKind::Hwb => index == 0,
        RelativeColorKind::Lch | RelativeColorKind::Oklch => index == 2,
        _ => false,
    }
}

pub(crate) fn relative_math_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "calc"
            | "min"
            | "max"
            | "clamp"
            | "sign"
            | "abs"
            | "round"
            | "mod"
            | "rem"
            | "pow"
            | "sqrt"
            | "hypot"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

fn relative_ident_math_type(
    kind: RelativeColorKind,
    _index: usize,
    name: &str,
) -> Option<ColorMathType> {
    if relative_ident_allowed(kind, name) {
        // Relative channel identifiers are exposed as unitless channel values.
        // A percentage or angle can still be produced by multiplying/dividing
        // them inside calc(), but it cannot be added directly to a number.
        Some(ColorMathType::Number)
    } else {
        None
    }
}

fn relative_math_expression_type<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    index: usize,
    allow_comma: bool,
) -> Result<ColorMathType, ParseError<'i, ()>> {
    let mut terms = Vec::new();
    loop {
        let token = match input.next() {
            Ok(token) => token.clone(),
            Err(_) => break,
        };
        match token {
            Token::Number { .. } => terms.push(MathTerm::Value(ColorMathType::Number)),
            Token::Percentage { .. } => terms.push(MathTerm::Value(ColorMathType::Percentage)),
            Token::Dimension { ref unit, .. } => {
                terms.push(MathTerm::Value(color_math_dimension_type(unit.as_ref())))
            }
            Token::Ident(ref name) if is_math_constant(name.as_ref()) => {
                terms.push(MathTerm::Value(ColorMathType::Number));
            }
            Token::Ident(ref name) => terms.push(MathTerm::Value(
                relative_ident_math_type(kind, index, name.as_ref())
                    .unwrap_or(ColorMathType::Invalid),
            )),
            Token::Function(ref name) => {
                let name_lower = name.as_ref().to_ascii_lowercase();
                let function_type = if name_lower == "var" || name_lower == "env" {
                    input.parse_nested_block(|nested| {
                        consume_math_component_values(nested)?;
                        Ok::<_, ParseError<'_, ()>>(ColorMathType::Unknown)
                    })?
                } else {
                    let nested_allows_comma = matches!(
                        name_lower.as_str(),
                        "min" | "max" | "clamp" | "round" | "mod" | "rem" | "atan2"
                    );
                    let nested = input
                        .parse_nested_block(|nested| {
                            relative_math_expression_type(nested, kind, index, nested_allows_comma)
                        })
                        .unwrap_or(ColorMathType::Invalid);
                    if !relative_math_function(name_lower.as_ref()) {
                        ColorMathType::Invalid
                    } else {
                        match name_lower.as_str() {
                            "calc" | "min" | "max" | "clamp" | "abs" | "round" | "mod" | "rem" => {
                                nested
                            }
                            "sign" | "pow" | "sqrt" | "hypot" | "log" | "exp" | "sin" | "cos"
                            | "tan" | "asin" | "acos" | "atan" | "atan2" => {
                                if nested == ColorMathType::Invalid {
                                    ColorMathType::Invalid
                                } else {
                                    ColorMathType::Number
                                }
                            }
                            _ => ColorMathType::Invalid,
                        }
                    }
                };
                terms.push(MathTerm::Value(function_type));
            }
            Token::ParenthesisBlock => {
                let nested = input
                    .parse_nested_block(|nested| {
                        relative_math_expression_type(nested, kind, index, false)
                    })
                    .unwrap_or(ColorMathType::Invalid);
                terms.push(MathTerm::Value(nested));
            }
            Token::Delim('+' | '-' | '*' | '/') => {
                if let Token::Delim(operator) = token {
                    terms.push(MathTerm::Operator(operator));
                }
            }
            Token::Comma if allow_comma => terms.push(MathTerm::Comma),
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            _ => terms.push(MathTerm::Value(ColorMathType::Invalid)),
        }
    }
    Ok(MathTermsParser::new(terms).parse_all(allow_comma))
}

fn parse_relative_component<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    index: usize,
) -> Result<(), ParseError<'i, ()>> {
    let token = input.next()?.clone();
    let is_hue = relative_component_is_hue(kind, index);
    match token {
        Token::Ident(ref name) if name.eq_ignore_ascii_case("none") => Ok(()),
        Token::Ident(ref name) if relative_ident_allowed(kind, name.as_ref()) => Ok(()),
        Token::Number { .. } => Ok(()),
        Token::Percentage { .. } if !is_hue => Ok(()),
        Token::Dimension { ref unit, .. } if is_angle_unit(unit.as_ref()) && is_hue => Ok(()),
        Token::Function(ref name)
            if relative_math_function(name.as_ref())
                || name.eq_ignore_ascii_case("var")
                || name.eq_ignore_ascii_case("env") =>
        {
            let name_lower = name.as_ref().to_ascii_lowercase();
            let value = if name_lower == "var" || name_lower == "env" {
                input.parse_nested_block(|nested| {
                    consume_math_component_values(nested)?;
                    Ok::<_, ParseError<'_, ()>>(ColorMathType::Unknown)
                })?
            } else {
                let allows_comma = matches!(
                    name_lower.as_str(),
                    "min" | "max" | "clamp" | "round" | "mod" | "rem" | "atan2"
                );
                input
                    .parse_nested_block(|nested| {
                        relative_math_expression_type(nested, kind, index, allows_comma)
                    })
                    .map_err(|_| input.new_custom_error(()))?
            };
            let allowed = matches!(value, ColorMathType::Unknown)
                || if is_hue {
                    matches!(value, ColorMathType::Number | ColorMathType::Angle)
                } else {
                    matches!(value, ColorMathType::Number | ColorMathType::Percentage)
                };
            if allowed {
                Ok(())
            } else {
                Err(input.new_custom_error(()))
            }
        }
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_relative_var_origin<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let token = input.next()?.clone();
    let Token::Function(ref name) = token else {
        return Err(input.new_custom_error(()));
    };
    if !name.eq_ignore_ascii_case("var") {
        return Err(input.new_custom_error(()));
    }
    input.parse_nested_block(|nested| {
        let custom_name = nested.expect_ident()?.clone();
        if !custom_name.as_ref().starts_with("--") {
            return Err(nested.new_custom_error(()));
        }
        if nested.try_parse(|i| i.expect_comma()).is_ok() {
            // The fallback is a component-value list. Its eventual color
            // validity is context-dependent, but it must not be empty syntax.
            let fallback_start = nested.state();
            consume_math_component_values(nested)?;
            if nested.state().position() == fallback_start.position() {
                return Err(nested.new_custom_error(()));
            }
        }
        nested
            .expect_exhausted()
            .map_err(|_| nested.new_custom_error(()))?;
        Ok(())
    })?;
    Ok(ParsedColor::from_css_color(CssColor::BLACK))
}

fn parse_relative_origin<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin_start = input.state();
    if let Some(origin) = parse_color_float(input, color_mix_depth) {
        Ok(origin)
    } else {
        input.reset(&origin_start);
        parse_relative_var_origin(input)
    }
}

fn parse_relative_color_after_from<'i>(
    input: &mut Parser<'i, '_>,
    kind: RelativeColorKind,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let origin = parse_relative_origin(input, color_mix_depth)?;
    let kind = if matches!(kind, RelativeColorKind::ColorRgb) {
        let target = input.expect_ident()?.clone();
        relative_kind_for_color_space(target.as_ref()).ok_or_else(|| input.new_custom_error(()))?
    } else {
        kind
    };
    for index in 0..3 {
        parse_relative_component(input, kind, index)?;
    }
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_relative_component(input, kind, 3)?;
    }
    // The context-free style model cannot retain a relative expression or
    // resolve currentColor/variables. Returning the origin preserves the
    // existing bounded CssColor representation while the grammar above does
    // the important parse-time validation.
    Ok(origin)
}

/// `<color-space>` (CSS Color 4 §13.2 "Color Space for Interpolation"
/// <https://www.w3.org/TR/css-color-4/#color-interpolation-method>)。
/// `color-mix()`では、bounded sRGB modelへ変換できる interpolation-space
/// identifiers も構文上受理する。`hsl`/`hwb` は円筒座標で補間して sRGB へ戻し、
/// wide-gamut identifiers は既存の bounded sRGB fallback を使う。Gradient
/// callers は CSS Images 側の実装範囲を保つため、`hsl`/`hwb` と wide-gamut
/// spaces を別途拒否する。
///
/// `color-mix()` (本 type の元々の用途、[`parse_mix_color_space`]) と
/// CSS Images 4 gradient function 群の `in <color-space>
/// <hue-interpolation-method>?` 節 ([`GradientColorInterpolation`]、
/// [`parse_gradient_color_interpolation`]) で共有する — 両 host syntax が
/// 同じ exported `<color-space>` production を参照するため、1 つの enum で
/// 両方を賄う。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixColorSpace {
    /// `srgb` — gamma-encoded sRGB。CSS の legacy default interpolation
    /// space。
    Srgb,
    /// `srgb-linear` — linear-light sRGB。
    SrgbLinear,
    /// `hsl` — cylindrical HSL coordinates with hue interpolation。
    Hsl,
    /// `hwb` — cylindrical HWB coordinates with hue interpolation。
    Hwb,
    /// `lab` — CIE Lab (rectangular)。
    Lab,
    /// `lch` — CIE LCH (polar、[`HueInterpolationMethod`] を受理)。
    Lch,
    /// `oklab` — Oklab (rectangular)。
    Oklab,
    /// `oklch` — Oklch (polar、[`HueInterpolationMethod`] を受理)。
    Oklch,
}

/// `<hue-interpolation-method>` (CSS Color 4 §13.2
/// <https://www.w3.org/TR/css-color-4/#color-interpolation-method>) —
/// `[ shorter | longer | increasing | decreasing ] hue`。polar な
/// [`MixColorSpace`] (`Lch`/`Oklch`) に対してのみ意味を持ち、それ以外では
/// caller が reject する ([`parse_color_mix_function`]、
/// [`parse_gradient_color_interpolation`])。`color-mix()` と gradient の
/// `<color-interpolation-method>` で共有する理由は [`MixColorSpace`] と同じ。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HueInterpolationMethod {
    /// `shorter hue` — 短い方の弧で補間する。省略時のこの production 自体の
    /// default。
    Shorter,
    /// `longer hue` — 長い方の弧で補間する。
    Longer,
    /// `increasing hue` — hue 角度が単調増加する方向で補間する。
    Increasing,
    /// `decreasing hue` — hue 角度が単調減少する方向で補間する。
    Decreasing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParsedColorSpace {
    Srgb,
    SrgbLinear,
    Lab,
    Lch,
    Oklab,
    Oklch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LightnessBoundary {
    Lower,
    Upper,
}

impl LightnessBoundary {
    fn from_coordinates(space: ParsedColorSpace, lightness: f32) -> Option<Self> {
        match space {
            ParsedColorSpace::Lab | ParsedColorSpace::Lch if lightness <= 0.0 => Some(Self::Lower),
            ParsedColorSpace::Lab | ParsedColorSpace::Lch if lightness >= 100.0 => {
                Some(Self::Upper)
            }
            ParsedColorSpace::Oklab | ParsedColorSpace::Oklch if lightness <= 0.0 => {
                Some(Self::Lower)
            }
            ParsedColorSpace::Oklab | ParsedColorSpace::Oklch if lightness >= 1.0 => {
                Some(Self::Upper)
            }
            _ => None,
        }
    }

    fn srgb_coordinates(self) -> [f32; 3] {
        match self {
            Self::Lower => [0.0; 3],
            Self::Upper => [1.0; 3],
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColorCoordinates {
    pub(crate) first: f32,
    pub(crate) second: f32,
    pub(crate) third: f32,
    pub(crate) alpha: f32,
    pub(crate) polar_hue_missing: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ParsedColor {
    pub(crate) coordinates: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) space: ParsedColorSpace,
    pub(crate) lightness_boundary: Option<LightnessBoundary>,
    pub(crate) polar_hue_missing: bool,
}

impl ParsedColor {
    fn from_css_color(color: CssColor) -> Self {
        Self::from_coordinates(
            ParsedColorSpace::Srgb,
            [
                f32::from(color.r) / 255.0,
                f32::from(color.g) / 255.0,
                f32::from(color.b) / 255.0,
            ],
            f32::from(color.a) / 255.0,
        )
    }

    pub(crate) fn from_coordinates(
        space: ParsedColorSpace,
        coordinates: [f32; 3],
        alpha: f32,
    ) -> Self {
        Self {
            coordinates,
            alpha,
            space,
            lightness_boundary: None,
            polar_hue_missing: false,
        }
    }

    fn from_lab_coordinates(space: ParsedColorSpace, coordinates: [f32; 3], alpha: f32) -> Self {
        Self {
            coordinates,
            alpha,
            space,
            lightness_boundary: LightnessBoundary::from_coordinates(space, coordinates[0]),
            polar_hue_missing: false,
        }
    }

    fn from_interpolation_coordinates_with_boundary(
        space: ParsedColorSpace,
        coordinates: [f32; 3],
        alpha: f32,
        lightness_boundary: Option<LightnessBoundary>,
        polar_hue_missing: bool,
    ) -> Self {
        // A generated lightness value outside its nominal range is still raw
        // interpolation data. Only caller-supplied endpoint provenance may
        // request the CSS boundary mapping.
        let parsed = Self {
            coordinates,
            alpha,
            space,
            lightness_boundary,
            polar_hue_missing,
        };
        if lightness_boundary.is_some()
            || matches!(
                space,
                ParsedColorSpace::Lab
                    | ParsedColorSpace::Lch
                    | ParsedColorSpace::Oklab
                    | ParsedColorSpace::Oklch
            )
        {
            return parsed;
        }
        // Preserve the legacy float-sRGB path for in-gamut values; retain
        // interpolation coordinates only when final sRGB conversion is out
        // of gamut, so nested mixes do not clip them early.
        let srgb = parsed.to_srgb_for_interpolation();
        if srgb.iter().all(|component| (0.0..=1.0).contains(component)) {
            Self::from_coordinates(ParsedColorSpace::Srgb, srgb, alpha)
        } else {
            parsed
        }
    }

    pub(crate) fn to_css_color(self) -> CssColor {
        rgb_f32_to_css_color(self.to_srgb(), self.alpha)
    }

    pub(crate) fn to_srgb(self) -> [f32; 3] {
        // Boundary provenance requests CSS Color's fixed black/white mapping;
        // retained interpolation coordinates otherwise use unbounded conversion.
        if let Some(boundary) = self.lightness_boundary {
            return boundary.srgb_coordinates();
        }
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates,
            ParsedColorSpace::SrgbLinear => self.coordinates.map(srgb_encode),
            ParsedColorSpace::Lab => lab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_unbounded(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Oklch => {
                oklab_to_srgb_unbounded(polar_to_rectangular(self.coordinates))
            }
        }
    }

    fn to_srgb_for_interpolation(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates,
            ParsedColorSpace::SrgbLinear => self.coordinates.map(srgb_encode),
            ParsedColorSpace::Lab => lab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_unbounded(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_unbounded(self.coordinates),
            ParsedColorSpace::Oklch => {
                oklab_to_srgb_unbounded(polar_to_rectangular(self.coordinates))
            }
        }
    }

    pub(crate) fn to_srgb_linear(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => self.coordinates.map(srgb_decode),
            ParsedColorSpace::SrgbLinear => self.coordinates,
            ParsedColorSpace::Lab => lab_to_srgb_linear(self.coordinates),
            ParsedColorSpace::Lch => lab_to_srgb_linear(polar_to_rectangular(self.coordinates)),
            ParsedColorSpace::Oklab => oklab_to_srgb_linear(self.coordinates),
            ParsedColorSpace::Oklch => oklab_to_srgb_linear(polar_to_rectangular(self.coordinates)),
        }
    }

    pub(crate) fn to_lab(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => srgb_to_lab(self.coordinates),
            ParsedColorSpace::SrgbLinear => linear_srgb_to_lab(self.coordinates),
            ParsedColorSpace::Lab => self.coordinates,
            ParsedColorSpace::Lch => polar_to_rectangular(self.coordinates),
            ParsedColorSpace::Oklab => linear_srgb_to_lab(oklab_to_srgb_linear(self.coordinates)),
            ParsedColorSpace::Oklch => {
                linear_srgb_to_lab(oklab_to_srgb_linear(polar_to_rectangular(self.coordinates)))
            }
        }
    }

    pub(crate) fn to_oklab(self) -> [f32; 3] {
        match self.space {
            ParsedColorSpace::Srgb => srgb_to_oklab(self.coordinates),
            ParsedColorSpace::SrgbLinear => linear_srgb_to_oklab(self.coordinates),
            ParsedColorSpace::Lab => linear_srgb_to_oklab(lab_to_srgb_linear(self.coordinates)),
            ParsedColorSpace::Lch => {
                linear_srgb_to_oklab(lab_to_srgb_linear(polar_to_rectangular(self.coordinates)))
            }
            ParsedColorSpace::Oklab => self.coordinates,
            ParsedColorSpace::Oklch => polar_to_rectangular(self.coordinates),
        }
    }

    fn to_coordinates(self, space: MixColorSpace) -> ColorCoordinates {
        let (coordinates, polar_hue_missing) = match (self.space, space) {
            (ParsedColorSpace::Srgb, MixColorSpace::Srgb)
            | (ParsedColorSpace::SrgbLinear, MixColorSpace::SrgbLinear)
            | (ParsedColorSpace::Lab, MixColorSpace::Lab)
            | (ParsedColorSpace::Oklab, MixColorSpace::Oklab) => (self.coordinates, false),
            (ParsedColorSpace::Lch, MixColorSpace::Lch) => (
                normalize_polar(self.coordinates, false, self.polar_hue_missing),
                self.polar_hue_missing,
            ),
            (ParsedColorSpace::Oklch, MixColorSpace::Oklch) => (
                normalize_polar(self.coordinates, true, self.polar_hue_missing),
                self.polar_hue_missing,
            ),
            (_, MixColorSpace::Hsl) => {
                let coordinates = srgb_to_hsl(self.to_srgb_for_interpolation());
                (coordinates, coordinates[1] == 0.0)
            }
            (_, MixColorSpace::Hwb) => {
                let coordinates = srgb_to_hwb(self.to_srgb_for_interpolation());
                (coordinates, coordinates[1] + coordinates[2] >= 1.0)
            }
            (_, MixColorSpace::Srgb) => (self.to_srgb_for_interpolation(), false),
            (_, MixColorSpace::SrgbLinear) => (self.to_srgb_linear(), false),
            (_, MixColorSpace::Lab) => (self.to_lab(), false),
            (_, MixColorSpace::Lch) => {
                let coordinates = rectangular_to_polar(self.to_lab(), false);
                (coordinates, polar_hue_missing_for_space(false, coordinates))
            }
            (_, MixColorSpace::Oklab) => (self.to_oklab(), false),
            (_, MixColorSpace::Oklch) => {
                let coordinates = rectangular_to_polar(self.to_oklab(), true);
                (coordinates, polar_hue_missing_for_space(true, coordinates))
            }
        };
        let [first, second, third] = coordinates;
        ColorCoordinates {
            first,
            second,
            third,
            alpha: self.alpha,
            polar_hue_missing,
        }
    }
}

impl From<MixColorSpace> for ParsedColorSpace {
    fn from(space: MixColorSpace) -> Self {
        match space {
            MixColorSpace::Srgb => Self::Srgb,
            MixColorSpace::SrgbLinear => Self::SrgbLinear,
            MixColorSpace::Hsl | MixColorSpace::Hwb => Self::Srgb,
            MixColorSpace::Lab => Self::Lab,
            MixColorSpace::Lch => Self::Lch,
            MixColorSpace::Oklab => Self::Oklab,
            MixColorSpace::Oklch => Self::Oklch,
        }
    }
}

fn polar_to_rectangular([lightness, chroma, hue]: [f32; 3]) -> [f32; 3] {
    [lightness, chroma * hue.cos(), chroma * hue.sin()]
}

fn polar_hue_missing_for_space(is_oklab: bool, coordinates: [f32; 3]) -> bool {
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    coordinates[1] <= chroma_threshold
}

fn normalize_polar(
    [lightness, chroma, hue]: [f32; 3],
    is_oklab: bool,
    polar_hue_missing: bool,
) -> [f32; 3] {
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    if chroma <= chroma_threshold {
        [
            lightness,
            if polar_hue_missing { 0.0 } else { chroma },
            if polar_hue_missing {
                0.0
            } else {
                hue.rem_euclid(std::f32::consts::TAU)
            },
        ]
    } else {
        [lightness, chroma, hue.rem_euclid(std::f32::consts::TAU)]
    }
}

fn rectangular_to_polar([lightness, a, b]: [f32; 3], is_oklab: bool) -> [f32; 3] {
    let chroma = a.hypot(b);
    let chroma_threshold = if is_oklab { 0.000004 } else { 0.0015 };
    if chroma <= chroma_threshold {
        [lightness, 0.0, 0.0]
    } else {
        [
            lightness,
            chroma,
            b.atan2(a).rem_euclid(std::f32::consts::TAU),
        ]
    }
}

fn parse_color_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    // Bounded CSS Color 4 support: the two sRGB spaces are representable by
    // CssColor without widening the public value model. Wide-gamut predefined
    // spaces remain a follow-up because this parser stores only 8-bit sRGB.
    let color_space = input.expect_ident()?.clone();
    let first = parse_color_component(input, 1.0)?;
    let second = parse_color_component(input, 1.0)?;
    let third = parse_color_component(input, 1.0)?;
    let alpha = parse_optional_modern_alpha(input)?;

    let (space, coordinates) = match color_space.as_ref().to_ascii_lowercase().as_str() {
        // CSS Color 4 §10.1 says out-of-gamut `color()` components are valid
        // and retained for intermediate computations. Keep authored `srgb`
        // coordinates raw; generated color-mix() values use the same path.
        "srgb" => (ParsedColorSpace::Srgb, [first, second, third]),
        // CSS Color 4 §10.1 also retains out-of-gamut linear-sRGB
        // components. Keep them in their declared space so srgb-linear
        // interpolation sees the raw coordinates; serialization converts and
        // bounds them only at the final sRGB/u8 sink.
        "srgb-linear" => (ParsedColorSpace::SrgbLinear, [first, second, third]),
        // For css-color parsing coverage (WPT), accept other predefined
        // spaces as srgb fallback (treat coordinates as srgb). This allows
        // `none` vectors in those spaces to count as valid without
        // widening the public value model to 8-bit gamut for those spaces.
        "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020" | "prophoto-rgb" | "xyz"
        | "xyz-d50" | "xyz-d65" => (ParsedColorSpace::Srgb, [first, second, third]),
        _ => return Err(input.new_custom_error(())),
    };
    Ok(ParsedColor::from_coordinates(space, coordinates, alpha))
}

fn parse_lab_function<'i>(
    input: &mut Parser<'i, '_>,
    function: LabFunction,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let is_oklab = matches!(function, LabFunction::Oklab | LabFunction::Oklch);
    let lightness = parse_lightness(input, is_oklab)?;
    let component_scale = match function {
        LabFunction::Lab => 125.0,
        LabFunction::Lch => 150.0,
        LabFunction::Oklab | LabFunction::Oklch => 0.4,
    };
    let mut second = parse_color_component(input, component_scale)?;
    let third = if matches!(function, LabFunction::Lch | LabFunction::Oklch) {
        second = second.max(0.0);
        parse_hue(input)?
    } else {
        parse_color_component(input, component_scale)?
    };
    let alpha = parse_optional_modern_alpha(input)?;

    let (space, coordinates) = match function {
        LabFunction::Lab => (ParsedColorSpace::Lab, [lightness, second, third]),
        LabFunction::Lch => (ParsedColorSpace::Lch, [lightness, second, third]),
        LabFunction::Oklab => (ParsedColorSpace::Oklab, [lightness, second, third]),
        LabFunction::Oklch => (ParsedColorSpace::Oklch, [lightness, second, third]),
    };
    Ok(ParsedColor::from_lab_coordinates(space, coordinates, alpha))
}

fn parse_hsl_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    // CSS Color 4 keeps the legacy comma form alongside the modern
    // space-separated form. The legacy grammar requires percentage
    // saturation/lightness and does not permit `none`.
    let hue_is_none = input.try_parse(|i| i.expect_ident_matching("none")).is_ok();
    let hue = if hue_is_none { 0.0 } else { parse_hue(input)? };
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        if hue_is_none {
            return Err(input.new_custom_error(()));
        }
        let saturation = expect_percentage_stable(input)?.clamp(0.0, 1.0);
        input.expect_comma()?;
        let lightness = expect_percentage_stable(input)?.clamp(0.0, 1.0);
        let alpha = if input.try_parse(|i| i.expect_comma()).is_ok() {
            parse_alpha_value(input)?
        } else {
            1.0
        };
        return Ok(ParsedColor::from_coordinates(
            ParsedColorSpace::Srgb,
            hsl_to_srgb(hue, saturation, lightness),
            alpha,
        ));
    }

    let saturation = parse_hsl_percentage_or_number(input)?;
    let lightness = parse_hsl_percentage_or_number(input)?;
    let alpha = parse_optional_modern_alpha(input)?;
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        hsl_to_srgb(hue, saturation, lightness),
        alpha,
    ))
}

fn parse_hsl_percentage_or_number<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        Ok(0.0)
    } else if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        Ok(if kind == ColorMathType::Percentage {
            value.clamp(0.0, 1.0)
        } else {
            (value / 100.0).clamp(0.0, 1.0)
        })
    } else if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        Ok(pct.clamp(0.0, 1.0))
    } else {
        Ok((expect_number_stable(input)? / 100.0).clamp(0.0, 1.0))
    }
}

fn hsl_to_srgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0 {
        return [lightness; 3];
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let hue_to_channel = |mut t: f32| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        hue_to_channel(hue / (2.0 * std::f32::consts::PI) + 1.0 / 3.0),
        hue_to_channel(hue / (2.0 * std::f32::consts::PI)),
        hue_to_channel(hue / (2.0 * std::f32::consts::PI) - 1.0 / 3.0),
    ]
}

fn srgb_to_hsl(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|component| component.clamp(0.0, 1.0));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let lightness = (max + min) * 0.5;
    if delta == 0.0 {
        return [0.0, 0.0, lightness];
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_degrees = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    [hue_degrees.to_radians(), saturation, lightness]
}

fn srgb_to_hwb(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = rgb.map(|component| component.clamp(0.0, 1.0));
    let [hue, _, _] = srgb_to_hsl(rgb);
    [
        hue,
        rgb[0].min(rgb[1]).min(rgb[2]),
        1.0 - rgb[0].max(rgb[1]).max(rgb[2]),
    ]
}

fn hwb_to_srgb(hue: f32, whiteness: f32, blackness: f32) -> [f32; 3] {
    let whiteness = whiteness.clamp(0.0, 1.0);
    let blackness = blackness.clamp(0.0, 1.0);
    if whiteness + blackness >= 1.0 {
        let gray = whiteness / (whiteness + blackness);
        [gray; 3]
    } else {
        let scale = 1.0 - whiteness - blackness;
        hsl_to_srgb(hue, 1.0, 0.5).map(|channel| channel * scale + whiteness)
    }
}

fn parse_hwb_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    let hue = parse_hue(input)?;
    let whiteness = parse_hsl_percentage_or_number(input)?;
    let blackness = parse_hsl_percentage_or_number(input)?;
    let alpha = parse_optional_modern_alpha(input)?;
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        hwb_to_srgb(hue, whiteness, blackness),
        alpha,
    ))
}

/// Parse one `<color-stop>` with its optional percentage in either order.
///
/// CSS Color 4 permits `<percentage> <color>` as well as the existing
/// `<color> <percentage>` spelling, but a stop may contain only one percentage.
fn parse_color_mix_stop<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<(ParsedColor, Option<f32>), ParseError<'i, ()>> {
    let leading_percentage = input.try_parse(parse_mix_percentage).ok();
    let color =
        parse_color_float(input, color_mix_depth).ok_or_else(|| input.new_custom_error(()))?;
    let trailing_percentage = input.try_parse(parse_mix_percentage).ok();
    if leading_percentage.is_some() && trailing_percentage.is_some() {
        return Err(input.new_custom_error(()));
    }
    Ok((color, leading_percentage.or(trailing_percentage)))
}

/// `in <color-space> <hue-interpolation-method>?` (CSS Color 4 §13.2
/// "Color Space for Interpolation" — [`MixColorSpace`] doc参照) の共通
/// parse + validation。`<hue-interpolation-method>` は polar な
/// `<color-space>` にのみ許され、省略時は
/// [`HueInterpolationMethod::Shorter`]がdefault。`allow_hsl_hwb` は
/// `color-mix()` では true、gradient の
/// `<color-interpolation-method>` ([`parse_gradient_color_interpolation`])
/// では false とし、host grammar の実装範囲を保つ。
fn parse_color_interpolation_method<'i>(
    input: &mut Parser<'i, '_>,
    allow_hsl_hwb: bool,
) -> Result<(MixColorSpace, HueInterpolationMethod), ParseError<'i, ()>> {
    input.expect_ident_matching("in")?;
    let color_space = parse_mix_color_space(input, allow_hsl_hwb)?;
    let explicit_hue_method = input.try_parse(parse_hue_interpolation_method).ok();
    if explicit_hue_method.is_some()
        && !matches!(
            color_space,
            MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
        )
    {
        return Err(input.new_custom_error(()));
    }
    Ok((
        color_space,
        explicit_hue_method.unwrap_or(HueInterpolationMethod::Shorter),
    ))
}

fn parse_color_mix_function<'i>(
    input: &mut Parser<'i, '_>,
    color_mix_depth: usize,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    // CSS Color 5 defaults an omitted interpolation method to sRGB. Keep the
    // shared helper strict for gradient callers, but make this color-mix
    // shorthand explicit here (`color-mix(red, blue)`).
    let interpolation_method = input.try_parse(|i| parse_color_interpolation_method(i, true));
    let has_interpolation_method = interpolation_method.is_ok();
    let (color_space, hue_interpolation_method) =
        interpolation_method.unwrap_or((MixColorSpace::Srgb, HueInterpolationMethod::Shorter));
    if has_interpolation_method {
        input.expect_comma()?;
    }

    // The current CSS Color 5 grammar accepts one or more color stops. Keep
    // parsing until the enclosing function block is exhausted; a comma with
    // no following stop naturally becomes an error rather than being silently
    // accepted as a trailing comma.
    let mut stops = vec![parse_color_mix_stop(input, color_mix_depth)?];
    while input.try_parse(|i| i.expect_comma()).is_ok() {
        stops.push(parse_color_mix_stop(input, color_mix_depth)?);
    }

    // A calc()-derived stop percentage is syntactically valid but cannot be
    // evaluated in this context-free parser. Use a finite placeholder for the
    // interpolation bookkeeping and keep the deferred marker so a placeholder
    // zero cannot make an otherwise valid declaration fail at parse time.
    let has_deferred_percentage = stops
        .iter()
        .any(|(_, percentage)| percentage.is_some_and(|value| !value.is_finite()));
    let specified_sum: f32 = stops
        .iter()
        .filter_map(|(_, percentage)| percentage.filter(|value| value.is_finite()))
        .sum();
    let unspecified_count = stops
        .iter()
        .filter(|(_, percentage)| percentage.is_none())
        .count();
    let has_unspecified = unspecified_count != 0;
    let mut raw_weights = Vec::with_capacity(stops.len());
    if has_unspecified && specified_sum <= 1.0 {
        let remainder = (1.0 - specified_sum) / unspecified_count as f32;
        raw_weights.extend(stops.iter().map(|(_, percentage)| match percentage {
            Some(value) if value.is_finite() => *value,
            Some(_) => 1.0,
            None => remainder,
        }));
    } else {
        raw_weights.extend(stops.iter().map(|(_, percentage)| match percentage {
            Some(value) if value.is_finite() => *value,
            Some(_) => 1.0,
            None => 0.0,
        }));
    }
    let raw_sum: f32 = raw_weights.iter().sum();
    if raw_sum == 0.0 {
        return Err(input.new_custom_error(()));
    }
    // If every stop was explicitly assigned a total below 100%, the missing
    // portion is transparent. Otherwise normalize the interpolation weights
    // to the full color contribution.
    let alpha_multiplier = if !has_deferred_percentage && !has_unspecified && specified_sum < 1.0 {
        specified_sum
    } else {
        1.0
    };
    let weights: Vec<f32> = raw_weights.iter().map(|weight| weight / raw_sum).collect();

    let first = stops[0].0;
    let mut mixed = css_color_to_coordinates(first, color_space);
    let mut accumulated_weight = weights[0];
    for ((color, _), weight) in stops.iter().skip(1).zip(weights.iter().skip(1)) {
        let next = css_color_to_coordinates(*color, color_space);
        mixed = if hue_interpolation_method == HueInterpolationMethod::Shorter {
            mix_coordinates(mixed, next, accumulated_weight, *weight, color_space)
        } else {
            mix_coordinates_with_hue(
                mixed,
                next,
                accumulated_weight,
                *weight,
                color_space,
                hue_interpolation_method,
            )
        };
        accumulated_weight += *weight;
    }

    let lightness_boundary = if stops.len() == 2 {
        let first = css_color_to_coordinates(stops[0].0, color_space);
        let second = css_color_to_coordinates(stops[1].0, color_space);
        propagated_lightness_boundary(
            stops[0].0,
            stops[1].0,
            [weights[0], weights[1]],
            color_space,
            mixed.first,
            [first.first, second.first],
        )
    } else {
        None
    };
    let (parsed_space, coordinates, polar_hue_missing) = match color_space {
        // HSL/HWB are represented as cylindrical interpolation coordinates
        // while mixing, then converted back to the bounded sRGB model.
        MixColorSpace::Hsl => (
            ParsedColorSpace::Srgb,
            hsl_to_srgb(mixed.first, mixed.second, mixed.third),
            false,
        ),
        MixColorSpace::Hwb => (
            ParsedColorSpace::Srgb,
            hwb_to_srgb(mixed.first, mixed.second, mixed.third),
            false,
        ),
        _ => (
            color_space.into(),
            [mixed.first, mixed.second, mixed.third],
            mixed.polar_hue_missing,
        ),
    };
    Ok(ParsedColor::from_interpolation_coordinates_with_boundary(
        parsed_space,
        coordinates,
        mixed.alpha * alpha_multiplier,
        lightness_boundary,
        polar_hue_missing,
    ))
}

fn parse_hue_interpolation_method<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<HueInterpolationMethod, ParseError<'i, ()>> {
    let name = input.expect_ident()?.clone();
    let method = match name.as_ref().to_ascii_lowercase().as_str() {
        "shorter" => HueInterpolationMethod::Shorter,
        "longer" => HueInterpolationMethod::Longer,
        "increasing" => HueInterpolationMethod::Increasing,
        "decreasing" => HueInterpolationMethod::Decreasing,
        _ => return Err(input.new_custom_error(())),
    };
    input.expect_ident_matching("hue")?;
    Ok(method)
}

fn parse_mix_color_space<'i>(
    input: &mut Parser<'i, '_>,
    allow_hsl_hwb: bool,
) -> Result<MixColorSpace, ParseError<'i, ()>> {
    let name = input.expect_ident()?.clone();
    let name = name.as_ref().to_ascii_lowercase();
    if allow_hsl_hwb && name == "hsl" {
        return Ok(MixColorSpace::Hsl);
    }
    if allow_hsl_hwb && name == "hwb" {
        return Ok(MixColorSpace::Hwb);
    }
    match name.as_str() {
        "srgb" => Ok(MixColorSpace::Srgb),
        "srgb-linear" => Ok(MixColorSpace::SrgbLinear),
        "lab" => Ok(MixColorSpace::Lab),
        "lch" => Ok(MixColorSpace::Lch),
        "oklab" => Ok(MixColorSpace::Oklab),
        "oklch" => Ok(MixColorSpace::Oklch),
        // The bounded color model stores the result as sRGB. Accept the
        // remaining CSS Color 4 interpolation-space identifiers for syntax
        // coverage and use the same fallback as `parse_color_function` for
        // wide-gamut endpoint spaces.
        "a98-rgb" | "display-p3" | "display-p3-linear" | "rec2020" | "prophoto-rgb" | "xyz"
        | "xyz-d50" | "xyz-d65" => Ok(MixColorSpace::Srgb),
        _ => Err(input.new_custom_error(())),
    }
}

// ── Numeric-token NaN stabilization ───────────────────────────────────
//
// cssparser 0.37.0's tokenizer computes a `<number>`/`<percentage>`/
// `<dimension>` token's value in two floating-point steps rather than as
// the single formula CSS Syntax Level 3 §4.3.13 defines
// (<https://www.w3.org/TR/css-syntax-3/#convert-a-string-to-a-number>,
// `s·(i + f·10⁻ᵈ)·10^(t·e)`): first the mantissa `sign * (integral_part +
// fractional_part)` as an f64, then, only if an exponent was written,
// `value *= f64::powf(10., sign * exponent)` (`tokenizer.rs:1081` in the
// `cssparser` crate). For a zero-mantissa literal with a huge exponent
// (`0e999`), `f64::powf(10., 999.)` evaluates to `+Infinity` first, and
// IEEE 754 defines `0.0 * Infinity` as `NaN` — even though the spec
// formula's actual value for this input is exactly `0` (multiplying by
// zero, not by infinity, is what the formula does mathematically).
// `"0e999".parse::<f32>()` (Rust's own single-step, correctly-rounded
// string-to-float conversion) returns `0` directly, confirming the value
// is exact and in-range — the `NaN` this crate would otherwise observe is
// purely an artifact of the tokenizer's two-step evaluation order.
//
// The same collapse mirrors in the opposite direction: digit accumulation
// in the tokenizer's integral-part loop can itself overflow to
// `+Infinity` for a sufficiently long run of digits with no exponent
// written, and multiplying that by a sufficiently negative exponent's
// `10^exponent` (which underflows to `0.0`) hits `Infinity * 0.0` = `NaN`
// the same way, for a literal whose true value is small but nonzero.
//
// `stabilize_nan_numeric_value` recovers both directions identically,
// since it does not special-case which operand was zero — it simply
// re-parses the token's own raw source text. The three functions below it
// (`expect_number_stable`/`expect_percentage_stable`/`next_numeric_stable`)
// are the acquisition points every NaN-sensitive numeric-token consumer in
// this module routes through, so the recovery happens once, at the
// source, and every downstream `!is_nan()` guard (and every range check
// that incidentally depends on NaN comparing `false`, e.g.
// `parse_filter_amount`'s `v >= 0.0`) sees the spec-correct value
// directly, without needing a special case of its own — including
// `parse_grid_flex_res` (the `fr` unit), which acquires its
// `Token::Dimension` via `next_numeric_stable` just like the other
// numeric-token consumers.
//
// This grammar-mirroring is itself a forward-maintenance hazard worth
// naming: `numeric_token_prefix` below re-implements cssparser's
// `consume_numeric` number grammar by hand, and the workspace pins
// `cssparser = "0.37"` (a caret range, not an exact version) — if a future
// `cssparser` version this range admits changes the `<number-token>`
// grammar (e.g. adds a new numeric literal shape), `numeric_token_prefix`
// must be updated to match, or it will silently mis-split the raw text for
// that new shape.

/// Splits the `<number>` production (CSS Syntax Level 3 §4.3.12 "Consume a
/// number" <https://www.w3.org/TR/css-syntax-3/#consume-number>; see also
/// §4.1's railroad diagram
/// <https://www.w3.org/TR/css-syntax-3/#number-token-diagram> for the
/// visual grammar: optional sign, digits, optional `.` + digits, optional
/// `[eE]` + optional sign + digits) off the front of `raw`. A percentage's
/// trailing `%` or a dimension's trailing unit is not part of this
/// production and is left in the (discarded) remainder — this mirrors cssparser's own
/// `consume_numeric` grammar exactly, so it consumes the whole numeric
/// prefix, and only the numeric prefix, of any text cssparser itself
/// already tokenized as a `<number-token>`/`<percentage-token>`/
/// `<dimension-token>`; no unit-length bookkeeping is needed to find the
/// boundary.
fn numeric_token_prefix(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        // The two closing braces below (after `i = j;`) are reached every
        // time this `if`'s condition is true — a plain assignment
        // statement with no early return/break cannot skip past its own
        // block's closing brace. Every actual call to this function is
        // gated on `value.is_nan()` at the call site (`expect_number_stable`
        // et al.), and `NaN` can only arise from a numeric token that had a
        // written exponent with at least one digit (module doc above), so
        // both this `if` and its enclosing one are always true for every
        // call this crate makes — there is no reachable path through this
        // function where they are false. Despite that, `cargo-llvm-cov`
        // 0.8.7's line-level report shows 0 hits on both closing-brace
        // lines even though the `i = j;` line directly above executes on
        // every one of those calls: a closing brace immediately following
        // the last (non-control-flow) statement of a nested `if` block,
        // when every exercised call takes the identical path through it,
        // does not get its own incremented coverage region in this
        // toolchain — reproduced in isolation with a minimal function of
        // the same shape. No additional test changes what these two lines
        // report, since the statement they follow is already exercised.
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        } // cov:ignore: closing-brace coverage-instrumentation artifact, see comment above the `if` this closes
    } // cov:ignore: same closing-brace artifact, one nesting level out — see comment above
    &raw[..i]
}

/// Recovers a numeric token's value from its own raw source text when
/// cssparser's tokenizer computed `NaN` for it (module doc above). `raw`
/// must already be trimmed to the `<number>` production span
/// (`numeric_token_prefix`). Re-parsing with `f64::from_str` computes the
/// CSS Syntax §4.3.13 formula in one step, so the `0 * Infinity` /
/// `Infinity * 0` intermediate never arises.
///
/// On the (expected-unreachable, since every `raw` this module passes in
/// is text cssparser itself already validated as a `<number-token>`)
/// chance the reparse fails, the original `value` (still `NaN`) is
/// returned unchanged, so callers' existing `is_nan()`/range-check guards
/// keep rejecting it rather than silently substituting a wrong number.
/// When `value` isn't `NaN` to begin with, this is a no-op — every
/// ordinary numeric literal, including magnitude overflow to real
/// `+Inf`/`-Inf`, is bit-identical to before this function existed.
pub(crate) fn stabilize_nan_numeric_value(raw_number_text: &str, value: f32) -> f32 {
    if !value.is_nan() {
        return value;
    }
    raw_number_text
        .parse::<f64>()
        .map(|v| v as f32)
        .unwrap_or(value)
}

/// `stabilize_nan_numeric_value`'s counterpart for `Token::Percentage`'s
/// `unit_value` field — `raw_number_text` is the same `<number>`
/// production span (`numeric_token_prefix`, with the trailing `%` already
/// excluded), but the recovered magnitude is divided by `100.` before
/// returning, matching `Parser::expect_percentage`'s own `unit_value`
/// convention (`0%`..`100%` → `0.0`..`1.0`). Shared by `expect_percentage_stable`
/// and `next_numeric_stable`'s `Token::Percentage` arm so the `/ 100.`
/// convention lives in one place rather than being repeated at each call
/// site.
pub(crate) fn stabilize_nan_percentage_value(raw_number_text: &str, unit_value: f32) -> f32 {
    if !unit_value.is_nan() {
        return unit_value;
    }
    raw_number_text
        .parse::<f64>()
        .map(|v| (v / 100.0) as f32)
        .unwrap_or(unit_value)
}

/// `Parser::expect_number` wrapper that applies `stabilize_nan_numeric_value`
/// before returning. Every `<number>` acquisition in this module should
/// call this instead of `Parser::expect_number` directly.
///
/// `Parser::skip_whitespace` is called explicitly before capturing the
/// start position: `Parser::next` (which `Parser::expect_number` calls
/// internally) skips leading whitespace *and comments* before reading the
/// token, so without this the captured position could point at that
/// skipped run instead of the token's first byte, and `Parser::slice_from`
/// would return a slice `numeric_token_prefix` can't walk (e.g. a leading
/// space makes it return `""`). Calling `skip_whitespace` here is a no-op
/// for `Parser::next`'s own subsequent call to it (nothing is left to
/// skip), so this changes no parsing behavior, only where the position is
/// captured.
fn expect_number_stable<'i>(input: &mut Parser<'i, '_>) -> Result<f32, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let value = input.expect_number()?;
    Ok(if value.is_nan() {
        stabilize_nan_numeric_value(numeric_token_prefix(input.slice_from(start)), value)
    } else {
        value
    })
}

/// `Parser::expect_percentage` wrapper — same recovery and
/// whitespace/comment-skip rationale as `expect_number_stable`, but
/// re-derives the pre-`/100` magnitude from the raw text and re-applies
/// `Parser::expect_percentage`'s own `/ 100.` convention before returning,
/// so the result stays a normalized `unit_value` (`0%`..`100%` → `0.0`..
/// `1.0`) like the wrapped method's.
fn expect_percentage_stable<'i>(input: &mut Parser<'i, '_>) -> Result<f32, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let unit_value = input.expect_percentage()?;
    Ok(if unit_value.is_nan() {
        stabilize_nan_percentage_value(numeric_token_prefix(input.slice_from(start)), unit_value)
    } else {
        unit_value
    })
}

/// `Parser::next` wrapper for the raw `Token::Number`/`Token::Percentage`/
/// `Token::Dimension` matches in this module (`parse_length_value`,
/// `parse_angle`, `parse_hue`) — corrects the token's own numeric field in
/// place when it is `NaN` (module doc above), before the caller's `match`
/// ever sees it. Non-numeric tokens, and numeric tokens whose value isn't
/// `NaN`, pass through unchanged. Same whitespace/comment-skip rationale
/// as `expect_number_stable`.
fn next_numeric_stable<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<Token<'i>, BasicParseError<'i>> {
    input.skip_whitespace();
    let start = input.position();
    let token = input.next()?.clone();
    Ok(match token {
        Token::Number {
            has_sign,
            value,
            int_value,
        } if value.is_nan() => Token::Number {
            has_sign,
            value: stabilize_nan_numeric_value(
                numeric_token_prefix(input.slice_from(start)),
                value,
            ),
            int_value,
        },
        Token::Percentage {
            has_sign,
            unit_value,
            int_value,
        } if unit_value.is_nan() => Token::Percentage {
            has_sign,
            unit_value: stabilize_nan_percentage_value(
                numeric_token_prefix(input.slice_from(start)),
                unit_value,
            ),
            int_value,
        },
        Token::Dimension {
            has_sign,
            value,
            int_value,
            unit,
        } if value.is_nan() => Token::Dimension {
            has_sign,
            value: stabilize_nan_numeric_value(
                numeric_token_prefix(input.slice_from(start)),
                value,
            ),
            int_value,
            unit,
        },
        other => other,
    })
}

fn parse_mix_percentage<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input
        .try_parse(|i| parse_color_math_value(i, ColorMathContext::Percentage))
        .is_ok()
    {
        // The exact weight is deferred. The caller treats NaN as a valid
        // syntactic percentage and substitutes a bookkeeping weight.
        return Ok(f32::NAN);
    }
    let percentage = expect_percentage_stable(input)?;
    if (0.0..=1.0).contains(&percentage) {
        Ok(percentage)
    } else {
        Err(input.new_custom_error(()))
    }
}

fn propagated_lightness_boundary(
    color_one: ParsedColor,
    color_two: ParsedColor,
    weights: [f32; 2],
    space: MixColorSpace,
    mixed_lightness: f32,
    endpoint_lightness: [f32; 2],
) -> Option<LightnessBoundary> {
    let [weight_one, weight_two] = weights;
    let [first_lightness, second_lightness] = endpoint_lightness;
    // Do not infer provenance from a generated result's lightness. It can be
    // outside the nominal range while still requiring raw interpolation.
    if weight_one == 1.0 && weight_two == 0.0 {
        return boundary_for_interpolation_space(
            space,
            color_one.space,
            color_one.lightness_boundary,
            mixed_lightness,
        );
    }
    if weight_one == 0.0 && weight_two == 1.0 {
        return boundary_for_interpolation_space(
            space,
            color_two.space,
            color_two.lightness_boundary,
            mixed_lightness,
        );
    }
    let effective_one = color_one.alpha * weight_one;
    let effective_two = color_two.alpha * weight_two;
    match (effective_one == 0.0, effective_two == 0.0) {
        (false, true) => boundary_for_interpolation_space(
            space,
            color_one.space,
            color_one.lightness_boundary,
            mixed_lightness,
        ),
        (true, false) => boundary_for_interpolation_space(
            space,
            color_two.space,
            color_two.lightness_boundary,
            mixed_lightness,
        ),
        (false, false) => {
            let boundary = match (color_one.lightness_boundary, color_two.lightness_boundary) {
                (Some(first), Some(second))
                    if first == second
                        && boundary_endpoint_is_structural(
                            color_one,
                            space,
                            first,
                            first_lightness,
                        )
                        && boundary_endpoint_is_structural(
                            color_two,
                            space,
                            first,
                            second_lightness,
                        ) =>
                {
                    Some(first)
                }
                (Some(boundary), None)
                    if boundary_endpoint_is_structural(
                        color_one,
                        space,
                        boundary,
                        first_lightness,
                    ) && boundary_endpoint_is_structural(
                        color_two,
                        space,
                        boundary,
                        second_lightness,
                    ) =>
                {
                    Some(boundary)
                }
                (None, Some(boundary))
                    if boundary_endpoint_is_structural(
                        color_one,
                        space,
                        boundary,
                        first_lightness,
                    ) && boundary_endpoint_is_structural(
                        color_two,
                        space,
                        boundary,
                        second_lightness,
                    ) =>
                {
                    Some(boundary)
                }
                _ => None,
            };
            boundary
                .filter(|boundary| lightness_boundary_matches(space, *boundary, mixed_lightness))
        }
        (true, true) => None,
    }
}

fn boundary_endpoint_is_structural(
    color: ParsedColor,
    space: MixColorSpace,
    boundary: LightnessBoundary,
    converted_lightness: f32,
) -> bool {
    if color.lightness_boundary.is_some() {
        return boundary_for_interpolation_space(
            space,
            color.space,
            Some(boundary),
            converted_lightness,
        ) == Some(boundary);
    }

    let target_is_lab = matches!(space, MixColorSpace::Lab | MixColorSpace::Lch);
    let target_is_oklab = matches!(space, MixColorSpace::Oklab | MixColorSpace::Oklch);
    let source_is_lab = matches!(color.space, ParsedColorSpace::Lab | ParsedColorSpace::Lch);
    let source_is_oklab = matches!(
        color.space,
        ParsedColorSpace::Oklab | ParsedColorSpace::Oklch
    );
    if source_is_lab && target_is_lab {
        let expected = match boundary {
            LightnessBoundary::Lower => 0.0,
            LightnessBoundary::Upper => 100.0,
        };
        let chroma = if matches!(color.space, ParsedColorSpace::Lch) {
            color.coordinates[1]
        } else {
            color.coordinates[1].hypot(color.coordinates[2])
        };
        return color.coordinates[0] == expected
            && chroma <= 0.0015
            && lightness_boundary_matches(space, boundary, converted_lightness);
    }
    if source_is_oklab && target_is_oklab {
        let expected = match boundary {
            LightnessBoundary::Lower => 0.0,
            LightnessBoundary::Upper => 1.0,
        };
        let chroma = if matches!(color.space, ParsedColorSpace::Oklch) {
            color.coordinates[1]
        } else {
            color.coordinates[1].hypot(color.coordinates[2])
        };
        return color.coordinates[0] == expected
            && chroma <= 0.000004
            && lightness_boundary_matches(space, boundary, converted_lightness);
    }
    if !matches!(
        color.space,
        ParsedColorSpace::Srgb | ParsedColorSpace::SrgbLinear
    ) {
        return false;
    }
    let expected = match boundary {
        LightnessBoundary::Lower => 0.0,
        LightnessBoundary::Upper => 1.0,
    };
    color
        .coordinates
        .iter()
        .all(|component| *component == expected)
        && lightness_boundary_matches(space, boundary, converted_lightness)
}

fn boundary_for_interpolation_space(
    space: MixColorSpace,
    source_space: ParsedColorSpace,
    boundary: Option<LightnessBoundary>,
    mixed_lightness: f32,
) -> Option<LightnessBoundary> {
    let source_is_lab = matches!(source_space, ParsedColorSpace::Lab | ParsedColorSpace::Lch);
    let source_is_oklab = matches!(
        source_space,
        ParsedColorSpace::Oklab | ParsedColorSpace::Oklch
    );
    let target_is_lab = matches!(space, MixColorSpace::Lab | MixColorSpace::Lch);
    let target_is_oklab = matches!(space, MixColorSpace::Oklab | MixColorSpace::Oklch);
    if !(source_is_lab && target_is_lab || source_is_oklab && target_is_oklab) {
        None
    } else {
        boundary.filter(|boundary| lightness_boundary_matches(space, *boundary, mixed_lightness))
    }
}

pub(crate) fn lightness_boundary_matches(
    space: MixColorSpace,
    boundary: LightnessBoundary,
    lightness: f32,
) -> bool {
    let expected = match (space, boundary) {
        (MixColorSpace::Lab | MixColorSpace::Lch, LightnessBoundary::Lower) => 0.0,
        (MixColorSpace::Lab | MixColorSpace::Lch, LightnessBoundary::Upper) => 100.0,
        (MixColorSpace::Oklab | MixColorSpace::Oklch, LightnessBoundary::Lower) => 0.0,
        (MixColorSpace::Oklab | MixColorSpace::Oklch, LightnessBoundary::Upper) => 1.0,
        (
            MixColorSpace::Srgb
            | MixColorSpace::SrgbLinear
            | MixColorSpace::Hsl
            | MixColorSpace::Hwb,
            _,
        ) => return false,
    };
    (lightness - expected).abs() <= 4.0 * f32::EPSILON * expected.abs().max(1.0)
}

fn parse_color_component<'i>(
    input: &mut Parser<'i, '_>,
    percentage_scale: f32,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value * percentage_scale);
    }
    if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(percentage * percentage_scale);
    }
    Ok(expect_number_stable(input)?)
}

fn parse_lightness<'i>(
    input: &mut Parser<'i, '_>,
    is_oklab: bool,
) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    let value = if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        if kind == ColorMathType::Percentage && !is_oklab {
            value * 100.0
        } else {
            value
        }
    } else if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        if is_oklab {
            percentage
        } else {
            percentage * 100.0
        }
    } else {
        expect_number_stable(input)?
    };
    Ok(if is_oklab {
        value.clamp(0.0, 1.0)
    } else {
        value.clamp(0.0, 100.0)
    })
}

pub(crate) fn is_angle_unit(unit: &str) -> bool {
    matches!(
        unit.to_ascii_lowercase().as_str(),
        "deg" | "grad" | "rad" | "turn"
    )
}

fn parse_hue<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if input
        .try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrAngle))
        .is_ok()
    {
        return Ok(0.0);
    }
    match next_numeric_stable(input)? {
        Token::Number { value, .. } => {
            let hue = value.rem_euclid(360.0).to_radians();
            if hue.is_finite() {
                Ok(hue)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        Token::Dimension {
            value, ref unit, ..
        } => {
            let hue = match unit.to_ascii_lowercase().as_str() {
                "deg" => value.rem_euclid(360.0).to_radians(),
                "grad" => (value.rem_euclid(400.0) * 0.9).to_radians(),
                "rad" => value.rem_euclid(std::f32::consts::TAU),
                "turn" => (value.rem_euclid(1.0) * 360.0).to_radians(),
                _ => return Err(input.new_custom_error(())),
            };
            if hue.is_finite() {
                Ok(hue)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        token => Err(input.new_unexpected_token_error(token)),
    }
}

fn parse_optional_modern_alpha<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_delim('/')).is_err() {
        return Ok(1.0);
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(percentage) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(percentage.clamp(0.0, 1.0));
    }
    Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
}

fn rgb_f32_to_css_color(rgb: [f32; 3], alpha: f32) -> CssColor {
    CssColor {
        r: channel_to_u8(rgb[0]),
        g: channel_to_u8(rgb[1]),
        b: channel_to_u8(rgb[2]),
        a: channel_to_u8(alpha),
    }
}

pub(crate) fn channel_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

// Keep transfer functions unclamped while colors are being converted for
// interpolation. The final `rgb_f32_to_css_color` conversion performs the
// destination sRGB gamut bound and 8-bit serialization.
pub(crate) fn srgb_encode(value: f32) -> f32 {
    let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
    let magnitude = value.abs();
    let encoded = if magnitude <= 0.0031308 {
        magnitude * 12.92
    } else {
        1.055 * magnitude.powf(1.0 / 2.4) - 0.055
    };
    sign * encoded
}

pub(crate) fn srgb_decode(value: f32) -> f32 {
    let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
    let magnitude = value.abs();
    let decoded = if magnitude <= 0.04045 {
        magnitude / 12.92
    } else {
        ((magnitude + 0.055) / 1.055).powf(2.4)
    };
    sign * decoded
}

// CSS Color conversion matrices are kept at their published precision; the
// runtime representation remains f32 until the final 8-bit property value.
fn lab_to_srgb_unbounded(coordinates: [f32; 3]) -> [f32; 3] {
    lab_to_srgb_linear(coordinates).map(srgb_encode)
}

#[allow(clippy::excessive_precision)]
fn lab_to_srgb_linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let fy = (lightness + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let epsilon = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let f_inv = |value: f32| {
        let cube = value * value * value;
        if cube > epsilon {
            cube
        } else {
            (116.0 * value - 16.0) / kappa
        }
    };
    let xyz_d50 = [
        0.9642956764 * f_inv(fx),
        f_inv(fy),
        0.8251046025 * f_inv(fz),
    ];
    let xyz_d65 = [
        0.9554734527 * xyz_d50[0] - 0.0230985369 * xyz_d50[1] + 0.0632593087 * xyz_d50[2],
        -0.0283697070 * xyz_d50[0] + 1.0099954580 * xyz_d50[1] + 0.0210413990 * xyz_d50[2],
        0.0123140017 * xyz_d50[0] - 0.0205076964 * xyz_d50[1] + 1.3303659366 * xyz_d50[2],
    ];
    [
        3.2409699 * xyz_d65[0] - 1.5373832 * xyz_d65[1] - 0.4986108 * xyz_d65[2],
        -0.9692436 * xyz_d65[0] + 1.8759675 * xyz_d65[1] + 0.0415551 * xyz_d65[2],
        0.0556301 * xyz_d65[0] - 0.2039769 * xyz_d65[1] + 1.0569715 * xyz_d65[2],
    ]
}

fn oklab_to_srgb_unbounded(coordinates: [f32; 3]) -> [f32; 3] {
    oklab_to_srgb_linear(coordinates).map(srgb_encode)
}

#[allow(clippy::excessive_precision)]
fn oklab_to_srgb_linear([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let l = lightness + 0.3963377774 * a + 0.2158037573 * b;
    let m = lightness - 0.1055613458 * a - 0.0638541728 * b;
    let s = lightness - 0.0894841775 * a - 1.2914855480 * b;
    let l = l * l * l;
    let m = m * m * m;
    let s = s * s * s;
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

pub(crate) fn css_color_to_coordinates(
    color: ParsedColor,
    space: MixColorSpace,
) -> ColorCoordinates {
    color.to_coordinates(space)
}

pub(crate) fn mix_coordinates(
    first: ColorCoordinates,
    second: ColorCoordinates,
    weight_one: f32,
    weight_two: f32,
    space: MixColorSpace,
) -> ColorCoordinates {
    mix_coordinates_with_hue(
        first,
        second,
        weight_one,
        weight_two,
        space,
        HueInterpolationMethod::Shorter,
    )
}

fn mix_coordinates_with_hue(
    first: ColorCoordinates,
    second: ColorCoordinates,
    weight_one: f32,
    weight_two: f32,
    space: MixColorSpace,
    hue_interpolation_method: HueInterpolationMethod,
) -> ColorCoordinates {
    let alpha_one = first.alpha * weight_one;
    let alpha_two = second.alpha * weight_two;
    let alpha = alpha_one + alpha_two;
    let component = |one: f32, two: f32| {
        if alpha == 0.0 {
            0.0
        } else {
            let weighted_one = if alpha_one == 0.0 {
                0.0
            } else {
                one * alpha_one
            };
            let weighted_two = if alpha_two == 0.0 {
                0.0
            } else {
                two * alpha_two
            };
            (weighted_one + weighted_two) / alpha
        }
    };
    let (second_component, third_component) = if matches!(
        space,
        MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
    ) {
        let hue = if first.polar_hue_missing != second.polar_hue_missing {
            if first.polar_hue_missing {
                second.third
            } else {
                first.third
            }
        } else if weight_one == 0.0 {
            second.third
        } else if weight_two == 0.0 {
            first.third
        } else {
            interpolate_hue(
                first.third,
                second.third,
                weight_two,
                hue_interpolation_method,
            )
        };
        (component(first.second, second.second), hue)
    } else {
        (
            component(first.second, second.second),
            component(first.third, second.third),
        )
    };
    let first_component = component(first.first, second.first);
    let polar_hue_missing = matches!(
        space,
        MixColorSpace::Hsl | MixColorSpace::Hwb | MixColorSpace::Lch | MixColorSpace::Oklch
    ) && first.polar_hue_missing
        && second.polar_hue_missing;
    ColorCoordinates {
        first: first_component,
        second: second_component,
        third: third_component,
        alpha,
        polar_hue_missing,
    }
}

fn interpolate_hue(first: f32, second: f32, progress: f32, method: HueInterpolationMethod) -> f32 {
    match method {
        // CSS Color 4 §13.5: keep theta2 - theta1 in `[-180, 180]`, retaining
        // the authored direction when the difference is exactly a half-turn.
        // Adjust the delta, as the legacy shorter path did, so wrapped results
        // keep their existing internal representative.
        HueInterpolationMethod::Shorter => {
            let mut delta = second - first;
            if delta > std::f32::consts::PI {
                delta -= std::f32::consts::TAU;
            } else if delta < -std::f32::consts::PI {
                delta += std::f32::consts::TAU;
            }
            first + delta * progress
        }
        // CSS Color 4 §13.5: keep theta2 - theta1 in (-360, -180] or
        // [180, 360), preferring a positive full turn when the angles match.
        HueInterpolationMethod::Longer => {
            let mut first = first;
            let mut second = second;
            let delta = second - first;
            if 0.0 < delta && delta < std::f32::consts::PI {
                first += std::f32::consts::TAU;
            } else if -std::f32::consts::PI < delta && delta <= 0.0 {
                second += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
        // CSS Color 4 §13.5: theta2 - theta1 in [0, 360).
        HueInterpolationMethod::Increasing => {
            let mut second = second;
            if second < first {
                second += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
        // CSS Color 4 §13.5: theta2 - theta1 in (-360, 0].
        HueInterpolationMethod::Decreasing => {
            let mut first = first;
            if first < second {
                first += std::f32::consts::TAU;
            }
            first + (second - first) * progress
        }
    }
}

fn srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    linear_srgb_to_lab(rgb.map(srgb_decode))
}

#[allow(clippy::excessive_precision)]
fn linear_srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    let xyz_d65 = [
        0.4123908 * rgb[0] + 0.3575843 * rgb[1] + 0.1804808 * rgb[2],
        0.2126390 * rgb[0] + 0.7151687 * rgb[1] + 0.0721923 * rgb[2],
        0.0193308 * rgb[0] + 0.1191948 * rgb[1] + 0.9505322 * rgb[2],
    ];
    let xyz_d50 = [
        1.0479298208 * xyz_d65[0] + 0.0229467933 * xyz_d65[1] - 0.0501922295 * xyz_d65[2],
        0.0296278157 * xyz_d65[0] + 0.9904344846 * xyz_d65[1] - 0.0170738250 * xyz_d65[2],
        -0.0092430582 * xyz_d65[0] + 0.0150551449 * xyz_d65[1] + 0.7518742814 * xyz_d65[2],
    ];
    let epsilon = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let f = |value: f32| {
        if value > epsilon {
            value.cbrt()
        } else {
            (kappa * value + 16.0) / 116.0
        }
    };
    let fx = f(xyz_d50[0] / 0.9642956764);
    let fy = f(xyz_d50[1]);
    let fz = f(xyz_d50[2] / 0.8251046025);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    linear_srgb_to_oklab(rgb.map(srgb_decode))
}

#[allow(clippy::excessive_precision)]
fn linear_srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let l = 0.41222147 * rgb[0] + 0.53633254 * rgb[1] + 0.05144599 * rgb[2];
    let m = 0.21190350 * rgb[0] + 0.68069955 * rgb[1] + 0.10739696 * rgb[2];
    let s = 0.08830246 * rgb[0] + 0.28171884 * rgb[1] + 0.62997870 * rgb[2];
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();
    [
        0.21045426 * l + 0.79361779 * m - 0.00407205 * s,
        1.97799850 * l - 2.42859221 * m + 0.45059371 * s,
        0.02590404 * l + 0.78277177 * m - 0.80867577 * s,
    ]
}

/// `border-*-color` の value parser — `currentcolor` keyword を先取りしてから
/// 既存 [`parse_color`] に委譲する。
///
/// CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
/// の border-*-color grammar は `<color>` そのもの、`<color>` production は
/// CSS Color 3 §4.4 <https://www.w3.org/TR/css-color-3/#currentColor-def>
/// `currentcolor` keyword を含む。しかし本 crate の [`parse_color`] は
/// cssparser の `parse_named_color` (RGB triple mapping)
/// 経由のため `currentcolor` は named-color table 未収載として `None` 側に
/// 落ちる — 本 helper が Ident 段で先取りする必要がある。resolution 委譲の
/// rationale は [`BorderColor`] enum doc 参照 (paint scope 責務)。
///
/// 5 call site (4 longhand + [`parse_border_shorthand`] color slot) が本
/// helper を経由する (sibling-arm convention consistency)。
fn parse_border_color(input: &mut Parser<'_, '_>) -> Option<BorderColor> {
    // `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、
    // sibling `parse_margin_side` line 1892 と同 shape の keyword intercept)。
    // 失敗時 `try_parse` が rewind、続く `parse_color` が Ident (named /
    // transparent) / Hash / Function の全 alternative を担当。
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(BorderColor::CurrentColor);
    }
    parse_color(input).map(BorderColor::Resolved)
}

/// `rgb()` / `rgba()` legacy comma syntax の中身 (関数呼び出しの括弧内) を
/// parse する。`parse_nested_block` の caller 側で `rgb(` / `rgba(` の function
/// token は既に consume 済み。`rgb` / `rgba` の function name は spec 上 alias
/// (CSS Color 4 §5.1: "rgb() and rgba() are now aliases for each other")
/// — alpha 省略は両者で許容し、name-based branching は行わない。
///
/// # Grammar (CSS Color 4 §5.1)
///
/// <https://www.w3.org/TR/css-color-4/#rgb-functions>
///
/// ```text
/// legacy-rgb-syntax  = rgb(  <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgba-syntax = rgba( <legacy-rgb-channel>#{3} , <alpha-value>? )
/// legacy-rgb-channel = <number> | <percentage>
/// alpha-value        = <number> | <percentage>
/// ```
///
/// legacy form の 3 channel は **all-number** or **all-percentage** の同一種で
/// なければならず、mix (`rgb(255, 50%, 0)`) は spec-invalid (§5.1:
/// "In the legacy form, the color channels can only be either all `<number>`s
/// or all `<percentage>`s — mixing types isn't allowed.")。
///
/// # Clamping
///
/// §5.1: "Values outside these ranges are not invalid, but are clamped to the
/// ranges defined here at parsed-value time" — 負値 / >255 (number) や
/// 100% 超も spec-valid、clamp only。
///
/// - `<number>` 0..=255 → `clamp_channel` で `i32.clamp(0, 255)` を 0..=1 に
///   正規化
/// - `<percentage>` 0%..=100% → `expect_percentage` は `0%`→0.0 / `100%`→1.0
///   の unit_value を返すため、そのまま normalized f32 として保持
/// - `<alpha-value>` は `<number>` 0..=1 または `<percentage>` 0%..=100% —
///   どちらも clamp 後に normalized f32 として保持
///
/// Legacy parser は color-mix() の endpoint を保持できるよう normalized f32 を返し、
/// property value へ落とす時だけ [`rgb_f32_to_css_color`] で u8 化する。
///
/// # Notes
///
/// - Modern (space + slash) syntax `rgb(R G B / A)` is parsed before the
///   legacy comma form. The two forms are not mixed.
/// - Fractional channels and position-aware `calc()` values are retained as
///   deferred syntax; the bounded parser uses zero placeholders until computed
///   value resolution is available.
/// - Alpha and channel `none` values are accepted syntactically and represented
///   as zero in the bounded model; missing-value carry-forward remains a later
///   computed-value concern.
fn parse_rgb_function<'i>(input: &mut Parser<'i, '_>) -> Result<ParsedColor, ParseError<'i, ()>> {
    // Try modern space-separated syntax first (CSS Color 4): `rgb(R G B [/ A])`
    // where each channel may be `none`, `<number>`, or `<percentage>`.
    // Modern syntax is space-separated, legacy is comma-separated.
    // We attempt modern via try_parse so legacy remains intact on failure.
    if let Ok(color) = input.try_parse(parse_modern_rgb_function) {
        return Ok(color);
    }
    // Legacy comma-separated path: `rgb(R, G, B [, A])` where all channels
    // share the same type (all numbers or all percentages).
    let (r, is_pct) = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        (pct.clamp(0.0, 1.0), true)
    } else if let Ok((kind, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        (
            if kind == ColorMathType::Percentage {
                value.clamp(0.0, 1.0)
            } else {
                clamp_rgb_number(value)
            },
            kind == ColorMathType::Percentage,
        )
    } else {
        (clamp_rgb_number(expect_number_stable(input)?), false)
    };
    input.expect_comma()?;
    let g = parse_rgb_channel(input, is_pct)?;
    input.expect_comma()?;
    let b = parse_rgb_channel(input, is_pct)?;
    // 4 番目 comma がある場合のみ alpha を parse。無ければ opaque (a=255)。
    // `rgba(...)` name 側で alpha 必須にしない (spec §5.1 alias 規定)。
    let a = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_alpha_value(input)?
    } else {
        1.0
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        [r, g, b],
        a,
    ))
}

fn parse_modern_channel<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(pct.clamp(0.0, 1.0));
    }
    // Modern number can be integer or float; use clamp_channel for ints and
    // float clamp for numbers.
    let n = expect_number_stable(input)?;
    Ok((n / 255.0).clamp(0.0, 1.0))
}

fn parse_modern_alpha<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(0.0);
    }
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        return Ok(value.clamp(0.0, 1.0));
    }
    if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        return Ok(pct.clamp(0.0, 1.0));
    }
    Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
}

fn parse_modern_rgb_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ParsedColor, ParseError<'i, ()>> {
    let r = parse_modern_channel(input)?;
    let g = parse_modern_channel(input)?;
    let b = parse_modern_channel(input)?;
    let a = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_modern_alpha(input)?
    } else {
        1.0
    };
    Ok(ParsedColor::from_coordinates(
        ParsedColorSpace::Srgb,
        [r, g, b],
        a,
    ))
}

/// legacy rgb() の 2 番目 / 3 番目 channel を parse する。1 番目 channel で
/// 決定した `is_pct` kind に沿って `<number>` / `<percentage>` のどちらかを
/// hard-expect し、mix (`rgb(255, 50%, 0)` / `rgb(50%, 255, 0)`) は Err で
/// 弾く (spec §5.1: "mixing types isn't allowed")。
fn parse_rgb_channel<'i>(
    input: &mut Parser<'i, '_>,
    is_pct: bool,
) -> Result<f32, ParseError<'i, ()>> {
    if is_pct {
        if let Ok((_, value)) =
            input.try_parse(|i| parse_color_math_value(i, ColorMathContext::Percentage))
        {
            return Ok(value.clamp(0.0, 1.0));
        }
        Ok(expect_percentage_stable(input)?.clamp(0.0, 1.0))
    } else {
        if let Ok((_, value)) =
            input.try_parse(|i| parse_color_math_value(i, ColorMathContext::Number))
        {
            return Ok(clamp_rgb_number(value));
        }
        Ok(clamp_rgb_number(expect_number_stable(input)?))
    }
}

/// `<alpha-value>` (CSS Color 4 §5.1 grammar: `<number> | <percentage>`)。
/// `<number>` は 0..=1、`<percentage>` は 0%..=100% で、どちらも clamp 後
/// normalized f32 に mapping する (`expect_percentage` の unit_value は既に
/// 0..=1 化されているため同一 formula)。
///
/// try_parse で percentage を先行させる — `<percentage>` は Token::Percentage、
/// `<number>` は Token::Number で orthogonal だが、percentage-first は
/// [`parse_rgb_function`] の 1 番目 channel と対称の順序 (mix reject と同じ
/// pattern で読める)。
fn parse_alpha_value<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    if let Ok((_, value)) =
        input.try_parse(|i| parse_color_math_value(i, ColorMathContext::NumberOrPercentage))
    {
        Ok(value.clamp(0.0, 1.0))
    } else if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        Ok(pct.clamp(0.0, 1.0))
    } else {
        Ok(expect_number_stable(input)?.clamp(0.0, 1.0))
    }
}

fn clamp_rgb_number(value: f32) -> f32 {
    (value / 255.0).clamp(0.0, 1.0)
}

/// `<opacity-value> = <number> | <percentage>` (CSS Color 4 §3.3
/// "Transparency: the opacity property"
/// <https://www.w3.org/TR/css-color-4/#transparency>). `<percentage>` uses
/// `expect_percentage`'s `unit_value` (already divided by 100) verbatim, no
/// 0%..100% range check.
///
/// # Deliberately does not clamp
///
/// Same §, verbatim: "Opacity values outside the range \[0, 1\] are not
/// invalid, and are preserved in specified values, but are clamped to the
/// range \[0, 1\] in computed values." Clamping is a **computed-value-time**
/// transform, so this parser — unlike [`parse_alpha_value`] (the `<alpha-value>`
/// grammar `rgb()`/`rgba()` use for their alpha channel, which shares the
/// exact same `<number> | <percentage>` grammar but clamps immediately at
/// parse time) — must preserve an out-of-range parse as-is. The two helpers
/// are kept separate rather than shared despite the identical grammar,
/// because sharing would silently store an already-clamped value at the
/// specified layer. The actual clamp lives in
/// [`crate::specified::SpecifiedValues::absolutize_with`] (phase 3) and its
/// page-context sibling.
///
/// # `!is_nan()` guard — NaN, but *not* `+Inf`/`-Inf`, must be rejected
///
/// `+Inf`/`-Inf` are spec-valid `<number>` values (CSS Color 4 §3.3 puts no
/// range restriction on `<opacity-value>`'s `<number>` alternative) and, per
/// CSS Values and Units Module Level 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>), when a literal
/// exceeds the implementation's supported precision it "must be converted to
/// the closest value supported by the implementation" where "how the
/// implementation defines 'closest' is implementation-defined" — treating
/// IEEE-754 `Infinity` as that closest value (rather than e.g. `f32::MAX`)
/// is a deliberate, spec-permitted choice, not the only conformant answer,
/// and is what cssparser's `f64`->`f32` overflow naturally produces. They are
/// handled correctly by the phase-3 clamp above — `f32::clamp` maps
/// `+Inf`/`-Inf` to `1.0`/`0.0` exactly as it maps any other out-of-range
/// finite value, so a huge-magnitude literal like `opacity: -1e40` (which
/// the tokenizer's f64->f32 conversion overflows to `-Infinity`, not NaN)
/// must reach the clamp unrejected and become `0.0` (fully transparent),
/// not fall back to the initial `1.0` (fully opaque) by being dropped
/// here. **NaN is different** — `f32::clamp` returns `self` unchanged when
/// `self` is NaN (only a NaN *bound* panics), so an unguarded NaN would
/// sail through the clamp and reach
/// [`crate::computed::ComputedValues::opacity`] — this guard is what keeps
/// that field NaN-free for values that go through this parser (see that
/// field's doc for the "ordinary parse -> cascade pipeline" scoping of
/// that guarantee). An earlier iteration of this guard used `is_finite()`,
/// which rejects `+Inf`/`-Inf` too and wrongly dropped `-1e40`-shaped
/// declarations — this helper deliberately does **not** reuse
/// [`parse_nonneg_finite_number`]'s `is_finite()` pattern for that reason;
/// unlike `flex-grow`/`flex-shrink`, which reject `<number [0,∞]>` and so
/// have no legitimate use for `-Inf` in the first place, `opacity`'s
/// unbounded `<number>` grammar makes `+Inf`/`-Inf` legitimate inputs that
/// must reach the clamp.
///
/// `expect_number_stable`/`expect_percentage_stable` already correct the
/// one class of `NaN` a numeric token can actually carry (a huge-exponent,
/// zero-mantissa literal like `opacity: 0e999`, module doc above) before
/// this function ever sees the value, so in ordinary use `n`/`pct` here
/// are never `NaN`. This `!is_nan()` check is kept as defense-in-depth —
/// it costs nothing when the value isn't `NaN` and still protects
/// [`crate::computed::ComputedValues::opacity`]'s NaN-free invariant if
/// that upstream recovery is ever bypassed or extended incorrectly.
fn parse_opacity_value(input: &mut Parser<'_, '_>) -> Option<f32> {
    let val = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        if pct.is_nan() {
            return None;
        }
        pct
    } else {
        let n = expect_number_stable(input).ok()?;
        if n.is_nan() {
            return None;
        }
        n
    };
    if input.try_parse(|i| i.expect_exhausted()).is_err() {
        return None;
    }
    Some(val)
}

/// `font-family: <family-name>#` を parse する。
///
/// comma-separated な family-name の list。各 family-name は quoted string
/// (`"Times New Roman"`) か、unquoted identifier の連続 (`Times New Roman` =
/// 3 ident が空白区切りで 1 family、CSS4 で有効) のいずれか。
///
/// 末尾で comma が続かなければ loop を止め、残り input (`!important` 等) は
/// 手を付けずに downstream (caller の `parse_important` / `expect_exhausted`)
/// に委ねる — `!` を garbage として拒否しないための Finding 3 対応。
fn parse_font_family(input: &mut Parser<'_, '_>) -> Option<Vec<Atom>> {
    let mut families = Vec::new();
    loop {
        // Try quoted string first (e.g. "Times New Roman")
        let family = if let Ok(s) = input.try_parse(|i| i.expect_string().cloned()) {
            Atom::from(s.as_ref())
        } else if let Ok(first) = input.try_parse(|i| i.expect_ident().cloned()) {
            // Unquoted ident sequence: `Times New Roman` = 3 idents joined by space
            let mut buf = first.as_ref().to_string();
            while let Ok(next) = input.try_parse(|i| i.expect_ident().cloned()) {
                buf.push(' ');
                buf.push_str(next.as_ref());
            }
            Atom::from(buf.as_str())
        } else {
            return None;
        };
        families.push(family);
        // Consume comma or stop (leaves remaining input alone)
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }
    if families.is_empty() {
        None
    } else {
        Some(families)
    }
}

/// `<length>` / `<length-percentage>` の共通 parser。1 token を consume する。
///
/// Grammar reference: CSS Values 4 §6 <https://www.w3.org/TR/css-values-4/#lengths>
/// / §5.5 <https://www.w3.org/TR/css-values-4/#percentages>.
///
/// # Mode selector
///
/// `allow_percentage` は `%` (`Token::Percentage`) token の受理有無のみを
/// 分岐する — dimension unit (`px` 等) の受理集合は分岐に依存しない (下の
/// `Token::Dimension` match arm 参照、両 mode で同一集合を受理する)。
/// - `false` → `<length>` mode: `%` を受理しない。
/// - `true` → `<length-percentage>` mode: `%` も受理する。
///
/// **受理 / 未対応 unit の一覧は本節では列挙しない** — 下の
/// `Token::Dimension` match arm (module doc 冒頭の「該 arm を single source
/// of truth として扱う」と同じ convention、`_` arm 直前 comment が未対応側の
/// 代表例を持つ) と `parse_length_value_rejects_unsupported_unit` test が
/// canonical。**ここに一覧を書き足す運用は受理 unit が増えるたびに drift
/// した** (実際に `cm` を筆頭に、`ch` / `ex` / `ic` /
/// `mm` / `in` / `pc` / `Q` / `lh` / `rlh` の一括拡張のたびに本節の一覧全体が
/// stale 化していた)。
///
/// # Unitless zero
///
/// CSS Values 3 §5 "Distance Units: the `<length>` type"
/// <https://www.w3.org/TR/css-values-3/#lengths> verbatim: "For zero lengths
/// the unit identifier is optional (i.e. can be syntactically represented as the
/// `<number>` 0)." — bare `0` (Token::Number, value == 0.0) を [`Length::Px`]
/// `(0.0)` として受理する (mode 非依存: `<length>` / `<length-percentage>` 両方)。
/// 非零 unitless number (`5`, `-1` etc.) は grammar 上 `<length>` にならないため
/// 引き続き drop する (`== 0.0` guard で判定)。
///
/// 同 spec §5 clause 2: "if a 0 could be parsed as either a `<number>` or a
/// `<length>` in a property (such as line-height), it must parse as a `<number>`"
/// — [`parse_line_height`] は本 helper より先に `expect_number` branch を試すため
/// 該当分岐は `LineHeight::Number(0.0)` を返し、本 helper 経由の `Length::Px(0.0)`
/// には落ちない (spec-required disambiguation)。
///
/// # Sign / range
///
/// 本 helper は sign / range check を行わない — property ごとに要件が異なるため
/// (padding は non-negative、margin は negative 許容、etc.)。caller 側で
/// post-filter する ([`parse_font_size`] は **全 [`Length`] variant** の payload に
/// 対して `>= 0.0` を確認する)。
///
/// # `allow_percentage=true` の caller
///
/// forward-provisioning として導入した mode だが、現在は 7 caller が使用する:
/// [`parse_margin_side`] / [`parse_padding_side`] / [`parse_width`] /
/// [`parse_height`] / [`parse_line_height`] / [`parse_font_size`] /
/// [`parse_text_indent`]。いずれも
/// grammar が spec で `<length-percentage>` を含む
/// (`font-size` は元は `<length>` 限定だったが後に拡張)。共通 helper 化により
/// 重複 dimension unit dispatch を回避している。
///
/// `allow_percentage=false` (= `<length>` mode) の caller は
/// [`parse_border_width_side`] / [`parse_letter_or_word_spacing`] —
/// 前者は CSS Backgrounds 3 §3.3 の `<line-width>` grammar が `<percentage>`
/// を含まないため、後者は CSS Text 3 §7.1/§7.2 の `letter-spacing` /
/// `word-spacing` grammar が共に "Percentages: N/A" と明記するため。
///
/// # Percentage overflow
///
/// `Token::Percentage.unit_value` は f64→f32 変換済 (cssparser 0.37
/// tokenizer が `value / 100.0` を emit) だが、[`Length::Percent`] は
/// authored number (`50%` → `50.0`) を保持する設計のため、本 helper 側で
/// `unit_value * 100.0` の逆変換を行う。`unit_value` 自体が f32 有限範囲に
/// 収まっていても (例 `1e40%` → cssparser 側は `1e38` で有限)、この
/// ×100.0 の逆変換それ自体が f32 overflow を起こしうる (`1e38 * 100.0` は
/// f32 の有限範囲 `3.4028235e38` を超えて `+Inf`)。CSS Values 4 §5 "Range
/// Checking and Precision for Numeric Types"
/// <https://www.w3.org/TR/css-values-4/#numeric-types> の "it must be
/// converted to the closest value supported by the implementation" に従い、
/// `±Inf` になった場合のみ、符号を保持しつつ `f32::MAX` へ寄せる。
///
/// 既存の sink-guard precedent (「guard は sink 境界に
/// 置く、parse/resolve 層には置かない」) はここには適用しない —
/// 本件は guard ではなく変換の正確さの問題
/// (specified 層の値そのものが CSS Values 4 §5 の要求から外れている)
/// であり、precedent とは別軸。`raikiri-dom::layout::sanitize_finite`
/// (resolve 後の geometry に対する sink guard) は本変更後も引き続き必要。
///
/// **`NaN` はこの saturation の対象外**。`is_finite()` は `NaN` に対しても
/// `false` を返すため、当初の実装は `NaN` も `±f32::MAX` へ saturate して
/// いたが、それは誤り: 例えば `0e999%` は cssparser の tokenizer が計算する
/// `0.0 * 10^999` (`f64::powf` が `+Inf` を返す) の中間結果としては `NaN`
/// になるが、**真の数学的値は 0** の入力であり、"closest value" は
/// `f32::MAX` ではなく `0.0` である。この関数が呼ぶ `next_numeric_stable`
/// (module doc 冒頭「Numeric-token NaN stabilization」節参照) がまさに
/// この class を token 取得の時点で訂正するため、通常の parse では
/// `unit_value` がこの arm に `NaN` のまま届くことはもう無く、`0e999%` は
/// この saturation 分岐を経由せずそのまま `Length::Percent(0.0)` になる。
/// それでも `NaN` を saturate 対象から除外する条件分岐 (`is_infinite()`
/// 限定) 自体は defense-in-depth として残す —
/// `sanitize_finite` (`raikiri-dom/src/layout.rs`) が `NaN` を既に `0.0`
/// として扱う既存の sink 契約と整合するため、`next_numeric_stable` の
/// recovery が (再 parse 失敗などで) 効かなかった残余の `NaN` も
/// `f32::MAX` へ寄せず無変換で通す。`is_infinite()` の saturation 自体は
/// `1e40%` のような正真正銘の magnitude overflow に対して引き続き
/// 必要 (module doc の「同じ collapse は逆方向にも起こりうる」とは別の、
/// 通常の overflow class)。
pub(crate) fn parse_length_value(
    input: &mut Parser<'_, '_>,
    allow_percentage: bool,
) -> Option<Length> {
    match &next_numeric_stable(input).ok()? {
        Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
            "px" => Some(Length::Px(*value)),
            "em" => Some(Length::Em(*value)),
            "rem" => Some(Length::Rem(*value)),
            "pt" => Some(Length::Pt(*value)),
            // Additional font-relative units (CSS Values 4 §6.1.1).
            // `ex`/`ch`/`ic` の real-metric variant は
            // style 層に font metrics が無いため常に spec fallback を使う
            // (`Length::Ex` / `Length::Ch` / `Length::Ic` の doc 参照)。
            "ex" => Some(Length::Ex(*value)),
            "rex" => Some(Length::Rex(*value)),
            "ch" => Some(Length::Ch(*value)),
            "rch" => Some(Length::Rch(*value)),
            "ic" => Some(Length::Ic(*value)),
            "ric" => Some(Length::Ric(*value)),
            // Additional absolute units (CSS Values 4 §6.2).
            // `unit` は `to_ascii_lowercase()` 済 —
            // `Q` トークンも `"q"` として届く。
            "cm" => Some(Length::Cm(*value)),
            "mm" => Some(Length::Mm(*value)),
            "q" => Some(Length::Q(*value)),
            "in" => Some(Length::In(*value)),
            "pc" => Some(Length::Pc(*value)),
            // `lh` / `rlh` (CSS Values 4 §6.1.1).
            // Accepted generally here for every consumer, `font-size` included
            // (moved out of `parse_font_size`'s former
            // post-filter — see that function's doc "`lh` / `rlh` は受理し、
            // 親基準で解決する" section for the self-reference resolution).
            "lh" => Some(Length::Lh(*value)),
            "rlh" => Some(Length::Rlh(*value)),
            // (b) 非対応 — viewport-relative unit (`vw`/`vh`/…) と
            // `cap`/`rcap` は未対応、silent drop。両者とも specified 層だけ
            // では正しく resolve できない (viewport size / font ascent が
            // style 層に存在しない) ため follow-up task へ切り出し済。
            //
            // この arm はそれ以外の全 unrecognized unit (例:
            // container-query unit `cqw`/`cqh`/`cqi`/`cqb`/`cqmin`/`cqmax` —
            // CSS Contain 3 §6 <https://www.w3.org/TR/css-contain-3/#container-lengths>、
            // container size も viewport size 同様 style 層に存在しない)
            // も等しく drop する。本 comment が「未対応 unit の一覧」の
            // canonical source になった以上、この一覧を書き足す形の
            // 重複記述はしないこと。
            _ => None,
        },
        Token::Percentage { unit_value, .. } if allow_percentage => {
            // authored-number 逆変換 + overflow saturation: 上の
            // "# Percentage overflow" section 参照。
            let percent = *unit_value * 100.0;
            Some(Length::Percent(if percent.is_infinite() {
                f32::MAX.copysign(percent)
            } else {
                percent
            }))
        }
        // CSS Values 3 §5 unitless-zero clause (doc "# Unitless zero" 参照)。
        Token::Number { value, .. } if *value == 0.0 => Some(Length::Px(0.0)),
        _ => None,
    }
}

/// [`Length`] の authored payload (`f32`) を variant によらず取り出す。
///
/// `parse_width` / `parse_font_size` / `parse_padding_side` /
/// `parse_border_width_side` / `parse_height` / `parse_line_height` /
/// [`parse_non_negative_length`] は grammar
/// の `[0,∞]` non-negative constraint を "全 variant の payload を取り出して
/// `>= 0.0` を確認" という同一 pattern で parse-time enforce する
/// (`parse_length_value` 自体は sign check しない仕様 — 同関数の "Sign / range"
/// doc 参照)。
///
/// 本 helper 導入前は 6 call site それぞれが `Length::Px(v) | Length::Em(v) |
/// … => v` の OR-pattern を個別に持っていた。
/// [`Length`] が 5 → 16 variant に増える際、6 site 全てを手で拡張すると
/// 1 か所でも変数を書き漏らした variant が非負チェックを素通りする
/// (実際 2 site — `parse_border_width_side` / `parse_line_height` — は
/// `_ => None` catch-all を持っていたため、拡張漏れは compile error にならず
/// 黙って新 unit を reject し続ける fail-quiet になっていた)。本 helper は
/// **exhaustive match を 1 か所に集約**することで、新 variant 追加時に
/// compile error で全 call site の見直しを強制する —
/// 「拡張のたびに N site 分の負債が乗る」パターンをこの関数の
/// 内側だけに閉じ込める。
fn length_payload(length: Length) -> f32 {
    match length {
        Length::Px(v)
        | Length::Em(v)
        | Length::Rem(v)
        | Length::Percent(v)
        | Length::Pt(v)
        | Length::Ex(v)
        | Length::Rex(v)
        | Length::Ch(v)
        | Length::Rch(v)
        | Length::Ic(v)
        | Length::Ric(v)
        | Length::Cm(v)
        | Length::Mm(v)
        | Length::Q(v)
        | Length::In(v)
        | Length::Pc(v)
        | Length::Lh(v)
        | Length::Rlh(v) => v,
    }
}

/// `<length [0,∞]>` — [`parse_length_value`] with `allow_percentage=false`
/// (no `<percentage>` alternative), then the same `[0,∞]` non-negative
/// filter [`length_payload`]'s doc describes (`(length_payload(length) >=
/// 0.0).then_some(length)`), so [`crate::page`]'s `size` descriptor parser
/// (the 7th caller in [`length_payload`]'s roster) doesn't have to
/// re-enumerate [`Length`] variants by hand.
///
/// `pub(crate)` for the one caller outside this module: [`crate::page`]'s
/// `size` descriptor parser. CSS Paged Media Level 3 §7.1 "Page size: the
/// size property" (<https://www.w3.org/TR/css-page-3/#page-size-prop>)
/// grammar is `<length>{1,2} | auto | …` — `<length>`, not
/// `<length-percentage>` — and states "Negative lengths are illegal", the
/// same `[0,∞]` shape this crate's box properties already enforce via
/// [`length_payload`].
pub(crate) fn parse_non_negative_length(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, false)?;
    (length_payload(length) >= 0.0).then_some(length)
}

/// `<length>` with no `<percentage>` alternative and no sign restriction —
/// [`parse_length_value`] with `allow_percentage=false`, unfiltered, so
/// callers outside this module don't have to re-enumerate [`Length`]
/// variants by hand.
///
/// `pub(crate)` for the one caller outside this module: [`crate::page`]'s
/// `bleed` descriptor parser. CSS Paged Media Level 3 §7.3 "Bleed Area: the
/// bleed property" (<https://www.w3.org/TR/css-page-3/#bleed>) grammar is
/// `auto | <length>` and explicitly permits negative values ("Values may be
/// negative, but there may be implementation-specific limits") — the same
/// unrestricted-sign shape [`parse_letter_or_word_spacing`] already uses for
/// `letter-spacing` / `word-spacing`, unlike this module's sibling
/// [`parse_non_negative_length`] (`size`'s `<length>` alternative, which the
/// spec instead states is `[0,∞]`).
pub(crate) fn parse_length_allow_negative(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_length_value(input, false)
}

/// `text-indent`'s full grammar.
///
/// grammar reference: CSS Text 3 §8.1
/// <https://www.w3.org/TR/css-text-3/#text-indent-property>, whose full
/// grammar is `<length-percentage> && hanging? && each-line?` — this helper
/// covers all three components, returning [`TextIndentValue`].
///
/// No non-negative filter, unlike [`parse_padding_side`] — the spec places no
/// `[0,∞]` restriction on this grammar (negative indents are valid, sibling
/// [`parse_margin_side`] applies the same "no filter" treatment for the same
/// reason its own grammar allows negative values).
fn parse_text_indent(input: &mut Parser<'_, '_>) -> Option<TextIndentValue> {
    // CSS Text 3 §8.1 grammar: `<length-percentage> && hanging? && each-line?`
    // Order-independent, but at least the length component must be present.
    // We collect optional hanging/each-line idents and one length-percentage,
    // in any order, then ensure no extra tokens.
    let mut length: Option<Length> = None;
    let mut hanging = false;
    let mut each_line = false;
    loop {
        // Try length-percentage (allow_percentage true)
        if length.is_none()
            && let Ok(l) = input.try_parse(|i| {
                parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(()))
            })
        {
            length = Some(l);
            continue;
        }
        // Try hanging
        if !hanging
            && input
                .try_parse(|i| i.expect_ident_matching("hanging"))
                .is_ok()
        {
            hanging = true;
            continue;
        }
        // Try each-line
        if !each_line
            && input
                .try_parse(|i| i.expect_ident_matching("each-line"))
                .is_ok()
        {
            each_line = true;
            continue;
        }
        break;
    }
    length.map(|length| TextIndentValue {
        length,
        hanging,
        each_line,
    })
}

/// `<length-percentage> | auto` の共通 parser — margin longhand 1 side 分。
///
/// grammar reference: CSS Box 3 §3.1
/// <https://www.w3.org/TR/css-box-3/#margin-physical> "Value:
/// `<length-percentage> | auto`"。
///
/// # Order of alternative
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部で
/// `input.next()` を unconditional に消費 (fail 時も token を戻さない) するため、
/// naive な "try length first, then auto" だと `margin: auto` の `auto` ident
/// が length parser で drop され後段の auto match が届かない。try_parse で
/// checkpoint 経由の rewind を確保する (sibling: [`parse_content_list_items`] の
/// bare `<string>` literal 分岐と同 pattern)。
///
/// `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、既存
/// `counter_reset_is_case_insensitive_on_none` test が挙動を pin) なので
/// `AUTO` / `Auto` も透過的に受理される。
pub(crate) fn parse_margin_side(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    parse_length_value(input, true).map(LengthOrAuto::Length)
}

/// `margin: <'margin-top'>{1,4}` shorthand — 1-4 value expansion 実装。
///
/// grammar reference: CSS Box 3 §3.2
/// <https://www.w3.org/TR/css-box-3/#margin-shorthand>。
///
/// # Expansion rules (spec verbatim, §3.2)
///
/// "If there is only one component value, it applies to all sides. If there
/// are two values, the top and bottom margins are set to the first value and
/// the right and left margins are set to the second. If there are three
/// values, the top is set to the first value, the left and right are set to
/// the second, and the bottom is set to the third. If there are four values
/// they apply to the top, right, bottom, and left, respectively."
///
/// # Trailing garbage handling
///
/// 5+ value (`margin: 10px 20px 30px 40px 50px`) は本 helper では 4 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`]
/// impl が `expect_exhausted` で余剰 token を
/// 検知して declaration ごと drop する (既存 [`parse_font_family`] 系と同じ
/// 責務分担、`rejects_extra_length_after_font_size` 系 test で pattern を pin)。
fn parse_margin_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<LengthOrAuto>> {
    let v1 = parse_margin_side(input)?;
    // 2nd value 不在 → 1 value case: 全 4 side に spread (§3.2 "If there is only
    // one component value, it applies to all sides")。
    let Some(v2) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides::all(v1));
    };
    // 3rd 不在 → 2 value case: top/bottom = 1st, right/left = 2nd。
    let Some(v3) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v1,
            left: v2,
        });
    };
    // 4th 不在 → 3 value case: top = 1st, right/left = 2nd, bottom = 3rd。
    let Some(v4) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(Sides {
            top: v1,
            right: v2,
            bottom: v3,
            left: v2,
        });
    };
    // 4 values: clockwise from top (top, right, bottom, left)。5th 以降は
    // 本 helper では消費せず、caller の `expect_exhausted` で drop される
    // (property.rs test `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
    //  で parse_value 単体挙動、rule.rs test `margin_shorthand_five_values_declaration_dropped`
    //  で end-to-end drop を pin)。
    Some(Sides {
        top: v1,
        right: v2,
        bottom: v3,
        left: v4,
    })
}

/// `margin-inline: <'margin-top'>{1,2}` / `margin-block: <'margin-top'>{1,2}`
/// shorthand を [`StartEnd<LengthOrAuto>`] に expand する — 1-2 value
/// expansion。
///
/// grammar reference: CSS Logical Properties and Values 1 §4.2
/// <https://www.w3.org/TR/css-logical-1/#propdef-margin-inline>: "The first
/// value represents the start edge style, and the second value represents
/// the end edge style. If only one value is given, it applies to both the
/// start and end edges." — `margin-block` の propdef も同じ文言・同じ
/// grammar (`<'margin-top'>{1,2}`) を共有するため、この 1 helper を
/// `margin-inline`/`margin-block` 両方の parser arm で共用する
/// ([`PropertyValue::MarginInline`] doc 参照 — 物理 axis (left/right か
/// top/bottom か) を決めるのは呼び出し側が選ぶ `PropertyValue` variant で
/// あり、この関数自体は axis を知らない)。
///
/// # Robustness
///
/// 3 個目以降の value は本 helper では consume せず leftover として残す →
/// caller (`rule.rs::DeclParser`) の `expect_exhausted` が declaration
/// ごと drop する ([`parse_margin_shorthand`] の "Trailing garbage
/// handling" 節と同型)。
fn parse_margin_logical_shorthand(input: &mut Parser<'_, '_>) -> Option<StartEnd<LengthOrAuto>> {
    let start = parse_margin_side(input)?;
    // 2nd value 不在 → 1 value case: start/end 両方に spread (spec "If only
    // one value is given, it applies to both the start and end edges")。
    let Some(end) = input.try_parse(|i| parse_margin_side(i).ok_or(())).ok() else {
        return Some(StartEnd::both(start));
    };
    Some(StartEnd { start, end })
}

/// `width: auto | <length-percentage [0,∞]>` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties> "Value:
/// `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`"、"Initial: auto"、"Inherited: no"。
///
/// # 非対応 (spec-valid、将来対応)
///
/// `min-content` / `max-content` / `fit-content()` は intrinsic sizing keyword
/// で未実装 — 本 helper では受理せず自然に `None` に落ちる (`auto` ident
/// 分岐で `expect_ident_matching("auto")` が fail、続く `parse_length_value` が
/// keyword / function token を Dimension / Percentage arm fall-through で drop)。
/// 負値 (`width: -10px`) は spec grammar `[0,∞]` violation として drop する。
///
/// # Order of alternatives
///
/// [`parse_margin_side`] と同 pattern の "auto ident branch 先行 try_parse":
/// [`parse_length_value`] は内部で `input.next()` を unconditional に消費する
/// (fail 時も token を戻さない) ため、naive な "try length first, then auto"
/// だと `width: auto` の `auto` ident が length parser で drop され後段の auto
/// match が届かない。`try_parse` で checkpoint 経由の rewind を確保する。
///
/// # Non-negative constraint
///
/// [`parse_padding_side`] と同 pattern の全 [`Length`] variant OR-pattern check —
/// spec `[0,∞]` の closed interval を parse-time enforce (Verification #4:
/// `width: -10px` → `None` → declaration drop)。padding と shape は同じだが
/// `auto` keyword 分岐が先行する (padding は `auto` を受理しない grammar
/// `<length-percentage [0,∞]>` のみ)。
fn parse_width(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `font-size: <absolute-size> | <relative-size> | <length-percentage [0,∞]> |
/// math` を parse する。
///
/// Grammar (CSS Fonts 4 §2.5 "Font size: the font-size property"
/// <https://www.w3.org/TR/css-fonts-4/#font-size-prop>):
/// `<absolute-size> | <relative-size> | <length-percentage [0,∞]> | math`。
///
/// - `<absolute-size>` (`xx-small` … `xxx-large`、`medium`) — [`parse_font_size_keyword`]
///   が §2.5.1 の scaling-factor table を `medium` = 16px 基準で解決し、
///   [`PropertyValue::FontSize`] (`Length::Px`) を返す。
/// - `<relative-size>` (`larger` / `smaller`) — 継承先依存のため
///   [`PropertyValue::FontSizeRelative`] を返し、解決は
///   [`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]
///   に委ねる (詳細は同 variant の doc)。
/// - `<length-percentage [0,∞]>` — 本関数の後半、[`parse_length_value`] 経由。
/// - `math` — 未実装 (MathML scaling algorithm が丸ごと未対応) として
///   `None` に落とす。
///
/// # ident 分岐を先に `try_parse` する理由
///
/// `<absolute-size>` / `<relative-size>` / `math` はいずれも単一 ident token。
/// [`parse_margin_side`] の `auto` 分岐と同じ pattern — [`parse_length_value`]
/// は内部で `input.next()` を unconditional に消費するため、ident 分岐は
/// checkpoint 経由の rewind (`try_parse`) で先に試す必要がある。
///
/// # `em` / `rem` / `%` / `pt` を受理するようになった経緯
///
/// 以前は `px` 以外を post-filter で drop していた。理由は「font-size
/// context resolve 未実装」であり、その resolve が後に実装された —
/// cascade が phase 2 で
/// [`crate::resolve::resolve_font_size`] を呼び、`em` は**親の** computed
/// font-size、`rem` は root element の computed font-size、`%` は同 §2.5
/// "Percentages: refer to parent element's font size" に従って絶対化する。
/// したがって drop の理由が消えたので受理する。
///
/// # Non-negative constraint
///
/// grammar の `[0,∞]` を parse-time enforce する。[`parse_padding_side`] /
/// [`parse_width`] と同じ [`length_payload`] 経由の全 [`Length`] variant check
/// — `-5px` だけでなく `-50%` / `-1em` も drop する。`<absolute-size>` /
/// `<relative-size>` は grammar 上そもそも符号を持たないので本 constraint の
/// 対象外 (ident 分岐は `parse_length_value` に達する前に return する)。
///
/// # `lh` / `rlh` は受理し、親基準で解決する
///
/// [`Length::Lh`] doc の「自己参照」節: CSS Values 4 §6.1.1 は `lh`/`rlh` が
/// `line-height` **または font-\* property** の値として、それが指す要素自身に
/// 使われたときは親 (または「親が無ければ initial values」) の line-height /
/// font metrics を基準にする、と規定する。`font-size` はまさにその
/// font-\* property であり、grammar 上 `lh`/`rlh` を排除する根拠は無い
/// (CSS Fonts 4 の `font-size` grammar `<absolute-size> | <relative-size> |
/// <length-percentage [0,∞]>` の `<length-percentage>` は `<length>` を含み、
/// CSS Values 4 §6.1.1 の `<length>` production は `lh`/`rlh` を除外しない)。
///
/// 当初は、この解決 (「親の computed line-height」を
/// font-size 解決の基準として渡す) が `line-height`
/// (`finalize`/`finalize_as_root` が既に持つ `parent: &ComputedValues` を
/// そのまま使える) より高コストに見えたため drop していたが、実際に実装した
/// ところコストは局所的だった — [`crate::resolve::resolve_font_size`] の
/// `self_reference_basis` 引数、および [`crate::specified::SpecifiedValues::finalize`]
/// 内の 2, 3 行の並べ替えで足りる (`parent` は本関数の呼び出しに入る前に
/// tree walk で既に確定済みのため、cross-node な phase 順序の変更は不要 —
/// [`mod@crate::resolve`] module doc の「想定される 4 段階」節参照)。
pub(crate) fn parse_font_size(input: &mut Parser<'_, '_>) -> Option<PropertyValue> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return parse_font_size_keyword(&ident);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(PropertyValue::FontSize(length))
}

/// `<absolute-size>` / `<relative-size>` / `math` の ident 部分を parse する
/// ([`parse_font_size`] の helper)。
///
/// # `<absolute-size>` scaling-factor table
///
/// CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping Table"
/// <https://www.w3.org/TR/css-fonts-4/#absolute-size-mapping> の表をそのまま
/// 写す (`resolve_relative_weight` の "算術式で書いてはいけない"
/// 方針と同じ理由 — 分数のまま持つことで丸め誤差の議論を spec 引用だけで
/// 閉じられる)。`medium` は raikiri の固定基準
/// ([`crate::computed::INITIAL_FONT_SIZE_PX`] = 16px、
/// [`crate::specified::SpecifiedValues::initial`] doc 参照) を再利用する:
///
/// | keyword | xx-small | x-small | small | medium | large | x-large | xx-large | xxx-large |
/// |---|---|---|---|---|---|---|---|---|
/// | factor | 3/5 | 3/4 | 8/9 | 1 | 6/5 | 3/2 | 2/1 | 3/1 |
///
/// 同 §の "an UA applying these guidelines should nevertheless avoid creating
/// font sizes of less than 9 device pixels per EM unit" は "should" (RFC 2119
/// 弱勧告)。本 table の最小値は `xx-small` = `16 * 3/5 = 9.6px` で、9px の
/// 下限を上回るため clamp は不要 (実装しない理由は「未対応」ではなく
/// 「`medium` = 16px 基準ではこの guideline を最初から満たす」こと)。
///
/// # `<relative-size>`
///
/// [`RelativeFontSize`] doc 参照。
///
/// # `math`
///
/// 未実装 (spec-valid だが対応外)。
fn parse_font_size_keyword(ident: &str) -> Option<PropertyValue> {
    const MEDIUM_PX: f32 = crate::computed::INITIAL_FONT_SIZE_PX;
    let px = match ident.to_ascii_lowercase().as_str() {
        "xx-small" => MEDIUM_PX * (3.0 / 5.0),
        "x-small" => MEDIUM_PX * (3.0 / 4.0),
        "small" => MEDIUM_PX * (8.0 / 9.0),
        "medium" => MEDIUM_PX,
        "large" => MEDIUM_PX * (6.0 / 5.0),
        "x-large" => MEDIUM_PX * (3.0 / 2.0),
        "xx-large" => MEDIUM_PX * (2.0 / 1.0),
        "xxx-large" => MEDIUM_PX * (3.0 / 1.0),
        "larger" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Larger)),
        "smaller" => return Some(PropertyValue::FontSizeRelative(RelativeFontSize::Smaller)),
        // `math` はここに落ちる (spec-valid だが未対応)。
        // 未知 ident も同じく drop。
        _ => return None,
    };
    Some(PropertyValue::FontSize(Length::Px(px)))
}

/// `padding-{top,right,bottom,left}` の single-side value を parse する。
///
/// grammar: `<length-percentage [0,∞]>` (CSS Box 3 §4.1
/// <https://www.w3.org/TR/css-box-3/#padding-physical>)。spec verbatim:
/// "Negative values for padding properties are invalid." — 負値は grammar 違反
/// として declaration ごと drop する。
///
/// # 実装 note
///
/// 1. [`parse_length_value`] を `allow_percentage=true` で呼ぶ (grammar が
///    `<length-percentage>`)。dimension 未対応 unit / `auto` keyword / non-numeric
///    token は同 helper が `None` に落とす (font-size 経路と同 pattern)。
/// 2. 全 [`Length`] variant の payload ([`length_payload`] 経由) に対し
///    `>= 0.0` を確認、負値は `None` 返し (`Percent(-10.0)` = `-10%` も含む —
///    Verification #5 で pin)。
///
/// # Sibling pattern
///
/// [`parse_font_size`] の `<length-percentage>` 分岐 (ident 分岐で `None` に
/// なった後の tail) と同形 — どちらも `allow_percentage=true` で
/// [`parse_length_value`] を呼び、[`length_payload`] で全 [`Length`] variant の
/// payload を抽出して `>= 0.0` を post-filter する (tail 部分の body は
/// identical)。`parse_font_size` は後に `<absolute-size>` /
/// `<relative-size>` / `math` の ident 分岐 (`parse_font_size_keyword`) が
/// 前段に付いたため関数全体としては同形ではなくなったが、この tail 部分の
/// ロジックは identical。
///
/// 両者が非対称だった時期 (font-size が `<length>` px-only scope で、padding
/// だけが `<length-percentage>` の 5 variant を受けていた頃) の記述は
/// font-relative unit 対応で解消済み。
fn parse_padding_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    (length_payload(length) >= 0.0).then_some(length)
}

/// `padding: <'padding-top'>{1,4}` shorthand を [`Sides<Length>`] に expand する。
///
/// CSS Box 3 §4.2 <https://www.w3.org/TR/css-box-3/#padding-shorthand> の
/// 1-4 value expansion (逐語引用ではないので `verbatim` 表記は使わない):
///
/// - 1 value: all 4 sides = value
/// - 2 values: top/bottom = 1st, left/right = 2nd
/// - 3 values: top = 1st, left/right = 2nd, bottom = 3rd
/// - 4 values: top / right / bottom / left (clockwise from top)
///
/// # Robustness
///
/// - 5 個目以降の value は本関数では consume せず leftover として残す →
///   caller (`rule.rs::DeclParser`) の `expect_exhausted` が declaration
///   ごと drop する (`padding: 1px 2px 3px 4px 5px` → invalid, drop)。
/// - 0 value (input が empty) は 1st `parse_padding_side` が `None` を返し
///   全体 `None` propagate。
/// - 各 value の non-negative constraint は [`parse_padding_side`] が個別に
///   enforce (負値混じり `padding: 10px -5px` → 2nd で `None`、全体 drop)。
fn parse_padding_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    // 1st value 必須。無ければ全体 drop (0-value form は grammar 違反)。
    let v1 = parse_padding_side(input)?;
    // 2-4 value は sequential `try_parse` で optional 取得。`try_parse` は
    // 失敗時に parser position を rewind するため、前段 None 時にも下段の
    // try_parse は同 token を再 read → 同 fail、guard 不要 (自然 short-circuit)。
    let v2 = input.try_parse(parse_padding_side_res).ok();
    let v3 = input.try_parse(parse_padding_side_res).ok();
    let v4 = input.try_parse(parse_padding_side_res).ok();
    // spec (CSS Box 3) §4.2 1-4 value expansion (code, not a spec quote):
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// [`parse_padding_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化。
fn parse_padding_side_res<'i>(input: &mut Parser<'i, '_>) -> Result<Length, ParseError<'i, ()>> {
    parse_padding_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `padding-inline: <'padding-top'>{1,2}` / `padding-block: <'padding-top'>{1,2}`
/// shorthand を [`StartEnd<Length>`] に expand する — 1-2 value expansion。
///
/// grammar reference: CSS Logical Properties and Values 1 §4.4
/// <https://www.w3.org/TR/css-logical-1/#propdef-padding-inline> — same
/// "first value = start, second value = end, one value spreads to both"
/// text as [`parse_margin_logical_shorthand`] documents in full for the
/// margin sibling; `padding-block`'s propdef shares the same grammar, so
/// this one helper backs both parser arms (axis choice is the caller's,
/// via which `PropertyValue` variant wraps the result).
///
/// # Robustness
///
/// 1st value の non-negative constraint 違反は [`parse_padding_side`] の
/// `?` propagation でそのまま `None` になる。2nd value 位置の違反は
/// `try_parse` が rewind するため **1-value form の `Some` として扱われ**、
/// 違反した token は unconsumed のまま残る → caller
/// (`rule.rs::DeclParser`) の `expect_exhausted` がその leftover を検知して
/// declaration ごと drop する ([`parse_padding_shorthand`] の "Robustness"
/// 節と同型 — `padding_shorthand_rejects_any_negative_value` test の doc が
/// 同じ shape を check する)。
fn parse_padding_logical_shorthand(input: &mut Parser<'_, '_>) -> Option<StartEnd<Length>> {
    let start = parse_padding_side(input)?;
    let end = input.try_parse(parse_padding_side_res).ok();
    Some(match end {
        None => StartEnd::both(start),
        Some(end) => StartEnd { start, end },
    })
}

/// `border-width` の `medium` keyword (= spec 上の initial value) に対応する
/// px 値。
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) 本文 verbatim:
/// "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." — `font-size` の `medium` (UA 裁量、
/// [`crate::computed::INITIAL_FONT_SIZE_PX`] 参照) とは異なり、こちらは
/// **spec が規範的に定める厳密値**であり、raikiri の選択ではない。
///
/// **非 test code で `3.0` (border-width `medium`) を書く単一 source**
/// ([`INITIAL_FONT_SIZE_PX`](crate::computed::INITIAL_FONT_SIZE_PX)
/// と同じ pattern) — [`parse_border_width_side`] の `medium` keyword 分岐と、
/// [`parse_border_shorthand`] の width 省略成分デフォルトが参照する。
/// [`crate::specified::INITIAL_BORDER`] の `width` field も本 const を参照する
/// (property → specified の既存依存方向 — `specified` は既に
/// `use crate::property::{..}` で本 module の型を import している。逆方向の
/// edge を作らないこと)。
///
/// 一方「initial の border-width が **3px そのものである**」ことの check は
/// test 側が literal で持つ。**これらを「一貫性のため」本 const への参照に
/// 書き換えてはならない** — 全体が自己参照になり、const の誤編集を何も
/// 検出できなくなる ([`INITIAL_FONT_SIZE_PX`](crate::computed::INITIAL_FONT_SIZE_PX)
/// doc と同じ理由)。該当 test は本 const を `5.0` 等に摂動すれば列挙できる
/// (lib test が fail-fast して doctest section まで到達しないので、
/// `cargo test -p raikiri-style` と `--doc` を別々に走らせること)。
///
/// **`thin` (1px) / `thick` (5px) は const 化しない** — 同じ規範文の 3 keyword
/// の残り 2 つだが、[`parse_border_width_side`] の keyword match 内 1 箇所ずつ
/// にしか現れず (border shorthand の省略成分デフォルトは spec 上も `medium`
/// のみが initial value)、複数 site 間の drift 余地がない。const 化するのは
/// 独立 literal が 2 箇所以上に分散している `medium` のみで十分
/// (`medium` の重複を解消する scope、thin/thick への一般化は
/// non-goal)。
pub(crate) const BORDER_WIDTH_MEDIUM_PX: f32 = 3.0;

/// `border-{top,right,bottom,left}-width` の single-side value を parse する。
///
/// Grammar: `<line-width>` = `<length [0,∞]> | thin | medium | thick`
/// (CSS Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。
/// **`<percentage>` は含まれない** — padding とは違う (
/// `parse_length_value(input, false)` = `<length>` mode を渡す)。
///
/// # Keyword mapping (spec 規定値)
///
/// spec §3.3 は 3 keyword を normative に規定する — verbatim: "The thin,
/// medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// respectively." 対応表:
/// - `thin`   → `Length::Px(1.0)`
/// - `medium` → `Length::Px(3.0)` (initial value)
/// - `thick`  → `Length::Px(5.0)`
///
/// UA 裁量ではなく spec 規定の equivalence なので、独立実装の制約下でも
/// そのまま採用できる (Chromium / Firefox / WebKit の実装とも一致)。
///
/// # Sign / range
///
/// spec `<length [0,∞]>` の non-negative 制約は本 helper が enforce する
/// (負値 → `None` = declaration drop)。sibling [`parse_padding_side`] と同じ
/// post-filter pattern だが、`Length::Percent` variant は生成されない
/// (`allow_percentage=false` により Percentage token 自体が reject される)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 負値 (`-1px`)、未知 keyword (`fat` 等)、
///   spec-invalid unit (`%` は grammar に含まれない → drop)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (将来対応)、silent drop。
fn parse_border_width_side(input: &mut Parser<'_, '_>) -> Option<Length> {
    // 1. keyword branch (thin / medium / thick) を先に try — `parse_length_value`
    //    は unconditional に token を consume するため、`try_parse` で rewind を
    //    確保する必要がある (sibling `parse_margin_side` の `auto` branch と同
    //    pattern)。
    let keyword = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.clone();
        match ident.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
            "thick" => Ok(Length::Px(5.0)),
            _ => Err(i.new_custom_error(())),
        }
    });
    if let Ok(l) = keyword {
        return Some(l);
    }
    // 2. `<length [0,∞]>` — allow_percentage=false で `<length>` mode
    //    (Percentage token は reject される、`<percentage>` は grammar 外)。
    let length = parse_length_value(input, false)?;
    // spec `<length [0,∞]>` の non-negative constraint — `length_payload` は
    // `Percent` も含む全 variant に対して定義されているが、`Percent` は
    // `allow_percentage=false` により本関数へは到達し得ない (unreachable、
    // dead value であって dead code ではない — helper 自体は border-width
    // 専用ではないため分岐を割ることはしない)。
    (length_payload(length) >= 0.0).then_some(length)
}

/// [`parse_border_width_side`] の `Result` 版 — `try_parse` は closure 内で
/// `Result` を要求するため wrapper 化 ([`parse_padding_side_res`] と同 pattern)。
fn parse_border_width_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_border_width_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-{top,right,bottom,left}-style` の single-side value を parse する。
///
/// Grammar: `<line-style>` = `none | hidden | dotted | dashed | solid | double
/// | groove | ridge | inset | outset` (CSS Backgrounds 3 §3.2
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>)。
/// ASCII case-insensitive で ident と照合 (sibling
/// [`parse_display`] / [`parse_text_align`] と同 flavor)。
///
/// # Non-goals
///
/// - **(a) spec-invalid → drop**: 未知 keyword (`wavy` 等) は silent drop。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
fn parse_border_style_side(input: &mut Parser<'_, '_>) -> Option<BorderStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(BorderStyle::None),
        "hidden" => Some(BorderStyle::Hidden),
        "dotted" => Some(BorderStyle::Dotted),
        "dashed" => Some(BorderStyle::Dashed),
        "solid" => Some(BorderStyle::Solid),
        "double" => Some(BorderStyle::Double),
        "groove" => Some(BorderStyle::Groove),
        "ridge" => Some(BorderStyle::Ridge),
        "inset" => Some(BorderStyle::Inset),
        "outset" => Some(BorderStyle::Outset),
        _ => None,
    }
}

/// `parse_border_style_side` の `Result` 版 (`try_parse` 用)。
fn parse_border_style_side_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderStyle, ParseError<'i, ()>> {
    parse_border_style_side(input).ok_or_else(|| input.new_custom_error(()))
}

/// `parse_border_color` の `Result` 版 (`try_parse` 用)。
fn parse_border_color_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BorderColor, ParseError<'i, ()>> {
    parse_border_color(input).ok_or_else(|| input.new_custom_error(()))
}

/// `border-style: <line-style>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。
/// 1-4 value expansion は [`parse_padding_shorthand`] と同型 (1 → all、
/// 2 → vertical/horizontal、3 → top/horizontal/bottom、4 → clockwise)。
/// 5 value 以降は caller の `expect_exhausted` が drop (padding precedent)。
fn parse_border_style_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<BorderStyle>> {
    let v1 = parse_border_style_side(input)?;
    let v2 = input.try_parse(parse_border_style_side_res).ok();
    let v3 = input.try_parse(parse_border_style_side_res).ok();
    let v4 = input.try_parse(parse_border_style_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border-width: <line-width>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。
/// 各 value の grammar は [`parse_border_width_side`] (thin/medium/thick +
/// 非負 `<length>`)、1-4 expansion は [`parse_padding_shorthand`] と同型。
fn parse_border_width_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Length>> {
    let v1 = parse_border_width_side(input)?;
    let v2 = input.try_parse(parse_border_width_side_res).ok();
    let v3 = input.try_parse(parse_border_width_side_res).ok();
    let v4 = input.try_parse(parse_border_width_side_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border-color: <color>{1,4}` shorthand (CSS Backgrounds 3 §3.4)。各
/// value の grammar は [`parse_border_color`] (`currentcolor` / named /
/// hash / function)、1-4 expansion は [`parse_padding_shorthand`] と同型。
fn parse_border_color_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<BorderColor>> {
    let v1 = parse_border_color(input)?;
    let v2 = input.try_parse(parse_border_color_res).ok();
    let v3 = input.try_parse(parse_border_color_res).ok();
    let v4 = input.try_parse(parse_border_color_res).ok();
    let sides = match (v2, v3, v4) {
        (None, _, _) => Sides::all(v1),
        (Some(h), None, _) => Sides {
            top: v1,
            right: h,
            bottom: v1,
            left: h,
        },
        (Some(h), Some(b), None) => Sides {
            top: v1,
            right: h,
            bottom: b,
            left: h,
        },
        (Some(r), Some(b), Some(l)) => Sides {
            top: v1,
            right: r,
            bottom: b,
            left: l,
        },
    };
    Some(sides)
}

/// `border: <line-width> || <line-style> || <color>` shorthand を parse する。
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>。
/// 4 side 全てに同一 [`Border`] を配る (`Sides::all`)。
///
/// # `||` (any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim:
/// "A double bar (||) separates two or more options: one or more of them must
/// occur, in any order." — 本 shorthand では:
/// - each component は最大 1 回 (2 回目の同 slot ident は spec-invalid = drop)
/// - at least 1 component が必須 (0 component の empty `border:` は drop)
/// - order は自由 (`1px solid red` / `red 1px solid` / `solid 1px` 全て valid)
///
/// # Loop 実装
///
/// unfilled slot (width / style / color) を loop で peel:
/// 1. `try_parse` で order-independent に各 slot の parser を試す
/// 2. 埋まっている slot に match する token に当たったら stop (spec 準拠、caller
///    の `expect_exhausted` が leftover を drop する — 例: `border: 1px 2px` は
///    `1px` を width に置いた後 `2px` は既に埋まっている width slot に match して
///    stop、caller が leftover を検出して declaration ごと drop)
/// 3. 全 slot が埋まった or どの parser も match しなくなったら break
/// 4. 少なくとも 1 slot が埋まっていれば `Some`、0 slot なら `None`
///
/// # Initial value fill (省略成分)
///
/// spec §3.4 verbatim: "Omitted values are set to their initial values."
/// 各成分の initial:
/// - width 省略 → `Length::Px(3.0)` (medium initial)
/// - style 省略 → `BorderStyle::None` (initial、spec §3.2)
/// - color 省略 → [`BorderColor::CurrentColor`] (spec §3.1 initial、used-value
///   resolution は paint scope 責務)
///
/// # Non-goals (spec deviation 明示)
///
/// spec §3.4 では border shorthand が **border-image-* も reset** する (spec
/// verbatim: "The border shorthand also resets border-image to its initial
/// value.") が、本 crate は border-image を実装していないため
/// reset side effect を省略。
/// border-image longhand 実装時に統合する。
///
/// # Sibling pattern
///
/// [`parse_margin_shorthand`] / [`parse_padding_shorthand`] は `{1,4}`
/// multiplier (順序固定、side ごとに違う値) だが、本 shorthand は `||` (any-order、
/// side は 4 side 共通) — 別 pattern。sibling は `try_parse` 経由の rewind と
/// initial fill の点で共通 principle を持つ。
pub(crate) fn parse_border_shorthand(input: &mut Parser<'_, '_>) -> Option<Sides<Border>> {
    let mut width: Option<Length> = None;
    let mut style: Option<BorderStyle> = None;
    let mut color: Option<BorderColor> = None;

    // `||` grammar: at least 1 component 必須、each component 最大 1 回、
    // order 自由。全 slot 満了 or 未 match token 到達で break。
    //
    // 各 iteration は "unfilled slot を順に try_parse、成功したら continue、
    // どの slot にも match しなかったら break" の shape。`continue` の前に slot
    // 満了 check を置くことで、埋まっている slot に対する 2 回目 (`border: 1px
    // 2px`) は自動的に fall-through して break (caller の `expect_exhausted` が
    // 残 token を検知して declaration drop)。
    loop {
        // 全 slot 満了 → break (leftover token は caller `expect_exhausted` が drop)
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }

        // width slot (unfilled のみ試行) — keyword (thin/medium/thick) と length
        // の両方を扱う helper を direct 呼ぶ。`try_parse` で失敗時 rewind。
        // `let Ok(..) = ..` の nested-if は clippy::collapsible-if を回避するため
        // let-chain (rust 1.88+) で 1 段化。
        if width.is_none()
            && let Ok(v) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(v);
            continue;
        }

        // style slot — ident が 10 keyword に match すれば埋める。`try_parse` で
        // 失敗時 rewind (width keyword `thin` / `medium` / `thick` を先に試すため
        // style keyword `none` / `solid` などとの間の ambiguity は無い、ident 集合が
        // disjoint)。
        if style.is_none()
            && let Ok(s) = input.try_parse(|i| -> Result<BorderStyle, ParseError<'_, ()>> {
                parse_border_style_side(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(s);
            continue;
        }

        // color slot — `parse_border_color` を reuse。hex / named / rgb(a) /
        // transparent の全 alternative + `currentcolor` keyword (CSS Color 3
        // §4.4) を受理。4 longhand parse site (border-{top,right,bottom,left}-color)
        // と同じ helper を経由することで sibling convention consistency を
        // 担保。
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<BorderColor, ParseError<'_, ()>> {
                parse_border_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }

        // どの unfilled slot にも match しなかった → 埋まっている slot に対する
        // 2 回目の指定 or 未知 token。break で loop 終了、caller の
        // `expect_exhausted` が leftover を drop する (`border: 1px 2px` →
        // `2px` は width slot 満了で本 fall-through 到達、declaration ごと drop)。
        break;
    }

    // spec `||` grammar: at least 1 component 必須。0 component (empty `border:`
    // or 未知 keyword only) は `None` = declaration drop。
    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    // 省略成分は spec §3.4 の initial value で埋める。
    let border = Border {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)), // medium
        style: style.unwrap_or(BorderStyle::None),
        // §3.1 initial "currentcolor" — used-value resolution は paint scope
        // 責務。
        color: color.unwrap_or(BorderColor::CurrentColor),
    };
    Some(Sides::all(border))
}

/// `height: <length-percentage [0,∞]> | auto` を parse する。
///
/// grammar reference: CSS Sizing 3 §3.1.1 "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>。value
/// grammar は `auto | <length-percentage [0,∞]> | min-content | max-content |
/// fit-content(<length-percentage>)`、initial value `auto`、Inheritance `No`。
///
/// # Scope carving
///
/// - **(a) spec-invalid → drop**: 負値 (`height: -10px`) は grammar `[0,∞]` 違反、
///   全 [`Length`] variant の payload に対し `>= 0.0` post-filter で reject
///   ([`parse_padding_side`] の非負フィルタ pattern と同 shape)。
/// - **(b) 非対応 — 未対応 sizing keyword**: `min-content` /
///   `max-content` / `fit-content(<length-percentage>)` は現状 scope
///   外、silent drop (auto ident branch から外れる他 keyword は
///   `expect_ident_matching("auto")` が失敗 → length parser の Dimension /
///   Percentage arm でも受理されず None に落ちる)。
/// - **(b) 非対応 — CSS-wide keyword**: 未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical。同 ident 経路で他 keyword と同じく
///   落ちる。旧稿は `all` を CSS-wide keyword の一つとして誤って列挙していた
///   — `all` は shorthand property 名であって値ではなく、この訂正も
///   consolidation の一部)。
/// - **calc() / var()**: 未実装、本 task scope 外
///   (`Token::Function` は `parse_length_value` が Dimension / Percentage 以外を
///   silent drop)。
///
/// # Order of alternative (sibling: [`parse_margin_side`])
///
/// `auto` ident branch を **先に** try_parse する — [`parse_length_value`] は内部
/// で `input.next()` を unconditional に消費するため、naive な "try length first,
/// then auto" だと `height: auto` の `auto` ident が length parser で drop され
/// 後段の auto match が届かない。`try_parse` で checkpoint 経由の rewind を
/// 確保する ([`parse_margin_side`] と同 pattern — margin の grammar `<length-
/// percentage> | auto` と同 shape を LengthOrAuto payload で共有)。
///
/// `expect_ident_matching` は ASCII case-insensitive (cssparser 慣行、既存
/// `counter_reset_is_case_insensitive_on_none` test が挙動を pin) なので
/// `AUTO` / `Auto` も透過的に受理される。
///
/// # Non-negative filter (sibling: [`parse_padding_side`])
///
/// spec `<length-percentage [0,∞]>` (§3.1.1) の非負制約は [`length_payload`]
/// 経由で全 [`Length`] variant の payload に対し `>= 0.0` を確認 —
/// [`parse_padding_side`] の同名 pattern を踏襲 (`<length-percentage [0,∞]>`
/// grammar と非負フィルタが対応する sibling)。`Percent(-10.0)` = `-10%` も
/// 含めて全 variant 経由で reject する。
fn parse_height(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `min-width` / `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` を parse する。
///
/// Grammar reference: CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>。initial value
/// `auto`、Inheritance `No`。sibling [`parse_max_size`] (CSS Sizing 3 §5) と
/// 同 shape で、`none` keyword 分岐が `auto` に置き換わる点だけが異なる
/// (min の initial は `auto`、`none` は max-only grammar)。
/// 非負制約 (`[0,∞]` → [`length_payload`] post-filter) と intrinsic keyword の
/// Auto placeholder mapping は sibling と同一。
fn parse_max_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `min-width` / `min-height: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content` を parse する.
///
/// Grammar reference: CSS Sizing 3 §4 "Minimum Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#min-size-properties>。initial value
/// `auto`、Inheritance `No`。sibling `parse_max_size` (CSS Sizing 3 §5) と
/// 同 shape で、`none` keyword 分岐が `auto` に置き換わる点だけが異なる
/// (min の initial は `auto`、`none` は max-only grammar)。
/// 非負制約 (`[0,∞]` → `length_payload` post-filter) と intrinsic keyword の
/// Auto placeholder mapping は sibling と同一。
fn parse_min_size(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        let _ = input.try_parse(|i| {
            i.expect_parenthesis_block()?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        });
        return Some(LengthOrAuto::Auto);
    }
    if input
        .try_parse(|i| {
            i.expect_function_matching("fit-content")?;
            i.parse_nested_block(|nested| {
                parse_length_value(nested, true)
                    .ok_or_else(|| nested.new_custom_error::<_, ()>(()))?;
                nested.expect_exhausted()?;
                Ok::<_, cssparser::ParseError<'_, ()>>(())
            })
        })
        .is_ok()
    {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

/// `line-height: normal | <number> | <length-percentage>` を parse する。
///
/// Grammar: CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#line-height-property>) — value
/// alternative は 3 branch:
///
/// 1. `normal` keyword → [`LineHeight::Normal`]
/// 2. `<number [0,∞]>` bare number (Token::Number、unit なし) → [`LineHeight::Number`]
/// 3. `<length-percentage [0,∞]>` → [`LineHeight::Length`] with reused Length variant
///
/// # Number vs Length grammar distinction
///
/// spec は `<number>` と `<length-percentage>` を別 alternative として持つため
/// 1 token レベルで区別が要る (Token::Number = unitless / Token::Dimension =
/// unit-bearing / Token::Percentage)。unitless `1.5` と dimensioned `1.5em` を
/// 別 variant に mapping することで、下流 (paint) が unitless number の
/// spec special behavior "specified value を child が inherit する"
/// (§5.1 "When a child element inherits a computed value...") と、length の
/// 通常 resolve context を区別できる。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → bare number
/// (`try_parse(|i| expect_number_stable(i))` — Dimension/Percentage に対しては rewind
/// して失敗) → [`parse_length_value`] (`allow_percentage = true`)。この順で
/// `1.5` は Number branch、`1.5em` / `1.5px` / `150%` は Length branch に確定分岐。
///
/// # Non-negative
///
/// spec `[0,∞]` により全 branch で negative reject:
/// - Number branch: `n >= 0.0` guard、負なら `None` = declaration drop
/// - Length branch: 全 payload の inner f32 に `>= 0.0` guard、負なら drop
///
/// spec-invalid → drop: spec grammar が range を parse-time
/// で制約するため、reject 自体が spec 準拠。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<number>` / `<length-percentage>` の負値、
///   `auto` / `medium` 等 spec-invalid keyword は spec grammar 違反、drop
fn parse_line_height(input: &mut Parser<'_, '_>) -> Option<LineHeight> {
    // 1. `normal` keyword — spec initial value。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LineHeight::Normal);
    }
    // 2. bare `<number [0,∞]>` — Token::Number (unit なし)。
    //    Dimension (`1.5em`) / Percentage (`150%`) に対しては `expect_number` が
    //    Err を返し `try_parse` が rewind するため、Length branch へフォールスルー。
    //    Number token を commit した後は必ずここで確定させる (accept か drop):
    //    `try_parse` は `Ok` の path で cursor を戻さないため、外側 `&& n >= 0.0`
    //    で reject すると consumed cursor のまま Length branch に落ち、
    //    `line-height: -0.5 20px` が `20px` として silently accept される
    //    (spec-invalid CSS を通す correctness bug)。
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // spec `<number [0,∞]>` 違反 → declaration drop (Length branch へ落とさない)。
        return (n >= 0.0).then_some(LineHeight::Number(n));
    }
    // 3. `<length-percentage [0,∞]>` — helper で全 unit + `%` を受理、
    //    negative は post-filter で drop (helper 自体は sign check しない仕様、
    //    parse_length_value doc "Sign / range" 参照)。
    let l = parse_length_value(input, true)?;
    // spec `[0,∞]`: 負値は grammar 違反 → declaration drop。
    (length_payload(l) >= 0.0).then_some(LineHeight::Length(l))
}

/// `tab-size: <number [0,∞]> | <length [0,∞]>` を parse する (CSS Text
/// Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// <https://www.w3.org/TR/css-text-3/#tab-size-property>)。
///
/// # Ordering
///
/// [`parse_line_height`] と同じ 2-branch shape (bare `<number>` を先に試し、
/// Dimension/Percentage には rewind して Length branch へ) から `normal`
/// branch を除いたもの — tab-size の grammar に `normal` alternative は無い。
/// `<length>` 側は `allow_percentage = false`
/// ([`parse_length_value`] — spec propdef "Percentages: N/A" が根拠、
/// [`TabSize::Length`] doc 参照)。
///
/// # Non-negative
///
/// spec `[0,∞]` (両 branch) — 全 branch で negative reject:
/// - Number branch: `n >= 0.0` guard、負なら `None` = declaration drop
/// - Length branch: payload の inner f32 に `>= 0.0` guard、負なら drop
///
/// [`parse_line_height`] doc の「Number token を commit した後は必ずここで
/// 確定させる」節と同じ懸念がここにも当てはまる — `try_parse` は `Ok` の
/// path で cursor を戻さないため、Number branch を通った後に外側で reject
/// すると `tab-size: -1 20px` が `20px` として silently accept されてしまう
/// (spec-invalid CSS を通す correctness bug)。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装、silent drop。
/// - **(a) spec-invalid → drop**: `<number>` / `<length>` の負値、`auto` 等
///   spec-invalid keyword、percentage は spec grammar 違反、drop。
fn parse_tab_size(input: &mut Parser<'_, '_>) -> Option<TabSize> {
    // 1. bare `<number [0,∞]>` — Token::Number (unit なし)。Dimension
    //    (`4px`) に対しては `expect_number` が Err を返し `try_parse` が
    //    rewind するため、Length branch へフォールスルー。
    if let Ok(n) = input.try_parse(|i| expect_number_stable(i)) {
        // spec `[0,∞]` 違反 → declaration drop (Length branch へ落とさない、
        // 上記 doc 節参照)。
        return (n >= 0.0).then_some(TabSize::Number(n));
    }
    // 2. `<length [0,∞]>` — percentage 非対応 (allow_percentage = false)。
    let l = parse_length_value(input, false)?;
    (length_payload(l) >= 0.0).then_some(TabSize::Length(l))
}

/// `letter-spacing: normal | <length>` / `word-spacing: normal | <length>`
/// を parse する。両 property は grammar が完全に同型 (CSS Text 3 §7.2
/// <https://www.w3.org/TR/css-text-3/#letter-spacing-property> / §7.1
/// <https://www.w3.org/TR/css-text-3/#word-spacing-property>) なので 1
/// 関数を共有する ([`LengthOrNormal`] doc の reuse pattern 節参照)。
///
/// # Ordering
///
/// `normal` (`try_parse` + `expect_ident_matching`) → [`parse_length_value`]
/// (`allow_percentage = false`) — [`parse_line_height`] と同じ 2-branch
/// shape だが、`<number>` branch が無い (grammar 自体に `<number>`
/// alternative が無いため、CSS Values 3 §5 の number-vs-length
/// disambiguation は本 property には適用されない — bare `0` はそのまま
/// [`parse_length_value`] の unitless-zero clause 経由で `Length::Px(0.0)`
/// になる)。
///
/// # Percentage は非対応
///
/// 両 property とも spec が "Percentages: N/A" (word-spacing) /
/// "Percentages: n/a" (letter-spacing) と明記する — `allow_percentage =
/// false` により `5%` は `_ => None` (Percentage token に対する
/// `allow_percentage` guard 不成立) で drop される。
///
/// # Negative length は許容 (non-negative filter を掛けない)
///
/// [`parse_line_height`] / [`parse_font_size`] 等の `[0,∞]` callers とは
/// 異なり、本関数は [`length_payload`] による `>= 0.0` post-filter を
/// **意図的に行わない**。CSS Text 3 §7.2 (letter-spacing) / §7.1
/// (word-spacing) がいずれも "Values may be negative, but there may be
/// implementation-dependent limits." と明記するため — spec 自身が sign を
/// 制限していない ([`LengthOrAuto`] を使う `margin-*` と同じ扱い、`padding`
/// / `border-width` の non-negative constraint とは対照的)。
///
/// # Non-goals
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(b) 非対応**: `calc()` / `var()` は未実装 (css-variables-and-math)、
///   silent drop
/// - **(a) spec-invalid → drop**: `<percentage>`、`auto` 等 spec-invalid
///   keyword は spec grammar 違反、drop
pub(crate) fn parse_letter_or_word_spacing(input: &mut Parser<'_, '_>) -> Option<LengthOrNormal> {
    // 1. `normal` keyword — spec initial value、"Computes to zero"。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LengthOrNormal::Normal);
    }
    // 2. `<length-percentage>` — CSS Text 4 adds percentage support
    //    (WPT letter-spacing-valid expects 120% / -10%). Sign not restricted.
    parse_length_value(input, true).map(LengthOrNormal::Length)
}

/// `flex-direction: row | row-reverse | column | column-reverse` を parse
/// する (CSS Flexible Box Layout Module Level 1 §5.1
/// <https://www.w3.org/TR/css-flexbox-1/#flex-direction-property>)。
/// [`parse_display`] と同じ single-ident ASCII case-insensitive idiom。
fn parse_flex_direction(input: &mut Parser<'_, '_>) -> Option<FlexDirectionValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirectionValue::Row),
        "row-reverse" => Some(FlexDirectionValue::RowReverse),
        "column" => Some(FlexDirectionValue::Column),
        "column-reverse" => Some(FlexDirectionValue::ColumnReverse),
        _ => None,
    }
}

/// `flex-wrap: nowrap | wrap | wrap-reverse` を parse する (CSS Flexible Box
/// Layout Module Level 1 §5.2
/// <https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property>)。
fn parse_flex_wrap(input: &mut Parser<'_, '_>) -> Option<FlexWrapValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrapValue::NoWrap),
        "wrap" => Some(FlexWrapValue::Wrap),
        "wrap-reverse" => Some(FlexWrapValue::WrapReverse),
        _ => None,
    }
}

/// `<number [0,∞]>` を parse する — `flex-grow` / `flex-shrink` 共有 helper。
///
/// # Non-negative **と** finite の両方を parse 時に enforce する
///
/// [`parse_line_height`] の `<number>` branch と同じ `[0,∞]` non-negative
/// check に加え、本 helper は **finiteness も** enforce する
/// ([`PropertyValue::FlexGrow`] doc 参照)。これは本 crate の他の length 系
/// helper (`parse_length_value` 等) とは非対称な判断で、理由は明示しておく:
/// `padding`/`width` 等の length は raikiri-dom 側の
/// `sanitize_taffy`/`sanitize_taffy_layout` という **sink 境界の guard** を
/// 必ず経由してから taffy に渡る (「guard は sink 境界に置く」という本 crate
/// 全体の設計方針、`crates/raikiri-dom/src/layout.rs` の非有限 f32 guard 節
/// 参照) ため parse 層では素通しでよい。一方 `flex-grow`/`flex-shrink` は
/// raikiri-dom の `bridge_flex` が `taffy::Style::flex_grow`/`flex_shrink`
/// (共に生 `f32`) へ **無変換で直接 copy する** — 途中に絶対化/sink guard の
/// 通過点が無いため、`+Inf` を防ぐ唯一の場所が本 parse-time check になる。
/// `<number [0,∞]>` 自体の f64→f32 変換 (cssparser tokenizer 側) は巨大な
/// literal (`flex-grow: 1e40`) で `+Inf` を produce しうる。`NaN` についても
/// 同じ `is_finite()` check が引き続き弾くが、`expect_number_stable`
/// (module doc「Numeric-token NaN stabilization」節参照) が zero-mantissa
/// huge-exponent literal (`flex-grow: 0e999`) 由来の `NaN` を acquisition
/// 時点で既に訂正するため、通常の parse ではこの check が `NaN` を実際に
/// 弾く場面はもう無い — `+Inf` に対する必須の check という位置づけ。
pub(crate) fn parse_nonneg_finite_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// [`parse_nonneg_finite_number`] の `Result` 版 — `try_parse` closure 用
/// ([`parse_padding_side_res`] と同じ wrapper pattern)。range/finite check
/// の失敗も `Err` として返すため、`try_parse` が呼び出し側で自動的に
/// rewind する (捕捉した Number token を別 branch へ fall through させない
/// — [`parse_line_height`] doc の「Number token を commit した後は必ず
/// ここで確定させる」節と同じ懸念を、check 自体を closure 内に置くことで
/// 構造的に回避する)。
fn parse_nonneg_finite_number_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    parse_nonneg_finite_number(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex-basis: content | <'width'>` を parse する (CSS Flexible Box Layout
/// Module Level 1 §7.2.3
/// <https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>)。
///
/// [`parse_width`] と同じ 3-branch shape (`auto` → `content` →
/// `<length-percentage [0,∞]>`) に `content` branch を追加したもの —
/// [`FlexBasisValue`] doc 参照。
pub(crate) fn parse_flex_basis(input: &mut Parser<'_, '_>) -> Option<FlexBasisValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(FlexBasisValue::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("content"))
        .is_ok()
    {
        return Some(FlexBasisValue::Content);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::MaxContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fit-content"))
        .is_ok()
    {
        return Some(FlexBasisValue::FitContent);
    }
    let length = parse_length_value(input, true)?;
    // `<'width'>` reuse: CSS Sizing 3 §3.1.1 の `[0,∞]` non-negative
    // constraint (`parse_width` と同 pattern)。
    (length_payload(length) >= 0.0).then_some(FlexBasisValue::Length(length))
}

/// [`parse_flex_basis`] の `Result` 版 ([`parse_padding_side_res`] と同じ
/// wrapper pattern、[`parse_flex_shorthand`] の `try_parse` 用)。
fn parse_flex_basis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexBasisValue, ParseError<'i, ()>> {
    parse_flex_basis(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]` を
/// parse する (CSS Flexible Box Layout Module Level 1 §7.1 "The flex
/// Shorthand" <https://www.w3.org/TR/css-flexbox-1/#flex-property>)。
///
/// [`FlexShorthand`] doc の "Omitted-component defaults" 節が説明する
/// shorthand-local default (grow=1 / shrink=1 / basis=0px、longhand 自身の
/// initial とは異なる) をここで適用する。
///
/// # `none` — exclusive keyword
///
/// spec §7.1 "The keyword none expands to 0 0 auto." — 他の component と
/// 共存しない (grammar top-level alternative)。
///
/// # Component order と unitless-zero ambiguity
///
/// grammar は `<flex-grow> <flex-shrink>?` の group と `<flex-basis>` の 2
/// group を `||` (any order、どちらか 1 つ以上) で combine する。本関数は
/// 最大 3 回のループで両 group を試す —
/// **`<flex-grow>` group を毎回先に試す**ことで、spec 本文の以下の
/// disambiguation 規則をそのまま実現する (verbatim):
///
/// > A unitless zero that is not already preceded by two flex factors must
/// > be interpreted as a flex factor. To avoid misinterpretation or invalid
/// > declarations, authors must specify a zero `<'flex-basis'>` component
/// > with a unit or precede it by two flex factors.
///
/// `grow` が未確定な間は bare `0` を常に number (flex factor) として先取り
/// consume するため、`flex: 0` は `grow=0` (`<'flex-basis'>` ではない) に
/// なる。`grow`/`shrink` が両方確定した**後**の bare `0` は
/// [`parse_flex_basis`] の unitless-zero clause 経由で `flex-basis` として
/// 解釈される (`flex: 2 3 0` → `basis: Length(Px(0.0))`)。
///
/// `<flex-shrink>` は grammar 上 `<flex-grow>` に直接後続する成分であり
/// (独立した `||` alternative ではない)、本関数もそれに合わせて `grow` を
/// 得た**直後**にのみ `shrink` を試す。
///
/// 5 個目以降の leftover token は本関数では consume せず、[`parse_padding_shorthand`]
/// 等と同じく caller (`rule.rs::DeclParser`) の `expect_exhausted` が
/// declaration ごと drop する。
pub(crate) fn parse_flex_shorthand(input: &mut Parser<'_, '_>) -> Option<FlexShorthand> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(FlexShorthand {
            grow: 0.0,
            shrink: 0.0,
            basis: FlexBasisValue::Auto,
        });
    }
    let mut grow: Option<f32> = None;
    let mut shrink: Option<f32> = None;
    let mut basis: Option<FlexBasisValue> = None;
    for _ in 0..3 {
        let mut progressed = false;
        if grow.is_none()
            && let Ok(g) = input.try_parse(parse_nonneg_finite_number_res)
        {
            grow = Some(g);
            // `<flex-shrink>` は `<flex-grow>` に直接後続する成分 — 独立
            // alternative としては試さない (上記 doc 参照)。
            if let Ok(s) = input.try_parse(parse_nonneg_finite_number_res) {
                shrink = Some(s);
            }
            progressed = true;
        }
        if !progressed
            && basis.is_none()
            && let Ok(b) = input.try_parse(parse_flex_basis_res)
        {
            basis = Some(b);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    // grammar 上どちらかの group が最低 1 つは要る — 0-value form
    // (`flex:` に何も続かない) は invalid。
    if grow.is_none() && basis.is_none() {
        return None;
    }
    Some(FlexShorthand {
        // shorthand-local default (`FlexShorthand` doc 参照) — longhand
        // 自身の initial (grow=0 / basis=auto) とは異なる。
        grow: grow.unwrap_or(1.0),
        shrink: shrink.unwrap_or(1.0),
        basis: basis.unwrap_or(FlexBasisValue::Length(Length::Px(0.0))),
    })
}

/// `order: <integer>` を parse する (CSS Flexible Box Layout Module Level 1
/// §4.2 "Display Order: the order property"
/// <https://www.w3.org/TR/css-flexbox-1/#order-property>、
/// [`PropertyValue::Order`] doc 参照)。
///
/// `expect_integer` 直接呼び出し — [`parse_z_index`] と同じ pattern。spec
/// value grammar は `<integer>` のみ (符号付き、range 制限なし)。
fn parse_order(input: &mut Parser<'_, '_>) -> Option<i32> {
    input.try_parse(|i| i.expect_integer()).ok()
}

/// [`parse_flex_direction`] の `Result` 版 ([`parse_padding_side_res`] と
/// 同じ wrapper pattern、[`parse_flex_flow`] の `try_parse` 用)。
fn parse_flex_direction_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexDirectionValue, ParseError<'i, ()>> {
    parse_flex_direction(input).ok_or_else(|| input.new_custom_error(()))
}

/// [`parse_flex_wrap`] の `Result` 版 ([`parse_flex_direction_res`] と同じ
/// wrapper pattern、[`parse_flex_flow`] の `try_parse` 用)。
fn parse_flex_wrap_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FlexWrapValue, ParseError<'i, ()>> {
    parse_flex_wrap(input).ok_or_else(|| input.new_custom_error(()))
}

/// `flex-flow: <'flex-direction'> || <'flex-wrap'>` を parse する (CSS
/// Flexible Box Layout Module Level 1 §5.3 "Flex Direction and Wrap: the
/// flex-flow shorthand"
/// <https://www.w3.org/TR/css-flexbox-1/#flex-flow-property>、
/// [`FlexFlow`] doc 参照)。
///
/// `||` (any-order、each component at most once、at least 1 必須) —
/// 最大 2 回のループで両 component を試す。省略成分は対応 longhand の
/// initial (direction=row / wrap=nowrap) を適用する。
/// leftover token は本関数では consume せず、caller
/// (`rule.rs::DeclParser`) の `expect_exhausted` が declaration ごと drop
/// する ([`parse_flex_shorthand`] と同じ contract)。
pub(crate) fn parse_flex_flow(input: &mut Parser<'_, '_>) -> Option<FlexFlow> {
    let mut direction: Option<FlexDirectionValue> = None;
    let mut wrap: Option<FlexWrapValue> = None;
    for _ in 0..2 {
        let mut progressed = false;
        if direction.is_none()
            && let Ok(d) = input.try_parse(parse_flex_direction_res)
        {
            direction = Some(d);
            progressed = true;
        }
        if !progressed
            && wrap.is_none()
            && let Ok(w) = input.try_parse(parse_flex_wrap_res)
        {
            wrap = Some(w);
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    if direction.is_none() && wrap.is_none() {
        return None;
    }
    Some(FlexFlow {
        direction: direction.unwrap_or(FlexDirectionValue::Row),
        wrap: wrap.unwrap_or(FlexWrapValue::NoWrap),
    })
}

/// `justify-content` / `align-content` 共有 parser
/// ([`ContentAlignmentValue`] doc の scope carving 節参照 — `safe`/`unsafe`
/// prefix、`<baseline-position>`、justify-content 独自の `left`/`right` は
/// 単一 ident しか consume しない本関数の shape 上、自然に unmatched (2-token
/// 列や非対応 ident は `_ => None`) になる)。
fn parse_content_alignment(input: &mut Parser<'_, '_>) -> Option<ContentAlignmentValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(ContentAlignmentValue::Normal),
        "stretch" => Some(ContentAlignmentValue::Stretch),
        "space-between" => Some(ContentAlignmentValue::SpaceBetween),
        "space-evenly" => Some(ContentAlignmentValue::SpaceEvenly),
        "space-around" => Some(ContentAlignmentValue::SpaceAround),
        "center" => Some(ContentAlignmentValue::Center),
        "start" => Some(ContentAlignmentValue::Start),
        "end" => Some(ContentAlignmentValue::End),
        "flex-start" => Some(ContentAlignmentValue::FlexStart),
        "flex-end" => Some(ContentAlignmentValue::FlexEnd),
        _ => None,
    }
}

/// [`parse_content_alignment`] の `Result` 版 ([`parse_padding_side_res`] と
/// 同じ wrapper pattern、[`parse_place_content_shorthand`] の `try_parse` 用)。
fn parse_content_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<ContentAlignmentValue, ParseError<'i, ()>> {
    parse_content_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// `align-items` parser ([`SelfAlignmentValue`] doc の scope carving 節
/// 参照)。`align-self` (`auto` を追加で受理する) は [`parse_align_self`] が
/// 本関数を再利用する。
fn parse_self_alignment(input: &mut Parser<'_, '_>) -> Option<SelfAlignmentValue> {
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    let ident = input.expect_ident().ok()?.clone();
    if ident.eq_ignore_ascii_case("last") {
        input.expect_ident_matching("baseline").ok()?;
        return Some(SelfAlignmentValue::Baseline);
    }
    let value = match ident.to_ascii_lowercase().as_str() {
        "normal" => SelfAlignmentValue::Normal,
        "stretch" => SelfAlignmentValue::Stretch,
        "center" => SelfAlignmentValue::Center,
        "start" | "self-start" => SelfAlignmentValue::Start,
        "end" | "self-end" => SelfAlignmentValue::End,
        "left" => SelfAlignmentValue::Start,
        "right" => SelfAlignmentValue::End,
        "flex-start" => SelfAlignmentValue::FlexStart,
        "flex-end" => SelfAlignmentValue::FlexEnd,
        "baseline" => SelfAlignmentValue::Baseline,
        _ => return None,
    };
    Some(
        if safe
            && matches!(
                value,
                SelfAlignmentValue::Center | SelfAlignmentValue::End | SelfAlignmentValue::FlexEnd
            )
        {
            SelfAlignmentValue::Start
        } else {
            value
        },
    )
}

/// [`parse_self_alignment`] の `Result` 版 ([`parse_padding_side_res`] と
/// 同じ wrapper pattern、[`parse_place_items_shorthand`] の `try_parse` 用)。
fn parse_self_alignment_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SelfAlignmentValue, ParseError<'i, ()>> {
    parse_self_alignment(input).ok_or_else(|| input.new_custom_error(()))
}

/// `align-self: auto | …` parser — `auto` branch を先に試し
/// (`try_parse` checkpoint、[`parse_width`] の "Order of alternatives" 節と
/// 同じ rationale)、それ以外は [`parse_self_alignment`] (= `align-items` と
/// 同じ keyword set) に delegate する。
fn parse_align_self(input: &mut Parser<'_, '_>) -> Option<AlignSelfValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(AlignSelfValue::Auto);
    }
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    parse_self_alignment(input).map(|value| {
        AlignSelfValue::Value(if safe && matches!(value, SelfAlignmentValue::Center) {
            SelfAlignmentValue::Start
        } else {
            value
        })
    })
}

fn parse_justify_self(input: &mut Parser<'_, '_>) -> Option<AlignSelfValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(AlignSelfValue::Auto);
    }
    let safe = input.try_parse(|i| i.expect_ident_matching("safe")).is_ok();
    let _unsafe = !safe
        && input
            .try_parse(|i| i.expect_ident_matching("unsafe"))
            .is_ok();
    parse_self_alignment(input).map(|value| {
        AlignSelfValue::Value(
            if safe
                && matches!(
                    value,
                    SelfAlignmentValue::Center
                        | SelfAlignmentValue::End
                        | SelfAlignmentValue::FlexEnd
                )
            {
                SelfAlignmentValue::Start
            } else {
                value
            },
        )
    })
}

/// [`parse_align_self`] の `Result` 版 ([`parse_padding_side_res`] と同じ
/// wrapper pattern、[`parse_place_self_shorthand`] の `try_parse` 用)。
fn parse_align_self_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AlignSelfValue, ParseError<'i, ()>> {
    parse_align_self(input).ok_or_else(|| input.new_custom_error(()))
}

/// `row-gap` / `column-gap`: `normal | <length-percentage [0,∞]>` 共有
/// parser (CSS Box Alignment Module Level 3 §8.1
/// <https://www.w3.org/TR/css-align-3/#propdef-row-gap>)。
///
/// [`parse_letter_or_word_spacing`] と shape は同じ (`normal` branch →
/// length branch) だが、**percentage を受理する**点が異なる
/// (`allow_percentage = true`、letter-spacing/word-spacing は "Percentages:
/// N/A" だが gap は `<length-percentage>`)。non-negative constraint は
/// [`parse_padding_side`] と同 pattern。
pub(crate) fn parse_gap_value(input: &mut Parser<'_, '_>) -> Option<LengthOrNormal> {
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(LengthOrNormal::Normal);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrNormal::Length(length))
}

/// [`parse_gap_value`] の `Result` 版 ([`parse_padding_side_res`] と同じ
/// wrapper pattern、[`parse_gap_shorthand`] の `try_parse` 用)。
fn parse_gap_value_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrNormal, ParseError<'i, ()>> {
    parse_gap_value(input).ok_or_else(|| input.new_custom_error(()))
}

/// `gap: <'row-gap'> <'column-gap'>?` shorthand を parse する (CSS Box
/// Alignment Module Level 3 §8.2
/// <https://www.w3.org/TR/css-align-3/#propdef-gap>)。第 2 成分省略時は
/// spec 本文通り第 1 成分をそのまま copy する ([`GapShorthand`] doc 参照)。
pub(crate) fn parse_gap_shorthand(input: &mut Parser<'_, '_>) -> Option<GapShorthand> {
    let row = parse_gap_value(input)?;
    let column = input.try_parse(parse_gap_value_res).unwrap_or(row);
    Some(GapShorthand { row, column })
}

/// `place-content: <'align-content'> <'justify-content'>?` shorthand を
/// parse する (CSS Box Alignment Module Level 3 §5.2
/// <https://www.w3.org/TR/css-align-3/#propdef-place-content>)。第 2 成分
/// 省略時は spec 本文通り第 1 成分をそのまま copy する
/// ([`PlaceContentShorthand`] doc 参照 — `<baseline-position>` 例外分岐は
/// [`ContentAlignmentValue`] が同 variant を持たないため本 crate では
/// 到達不能)。
pub(crate) fn parse_place_content_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<PlaceContentShorthand> {
    let align = parse_content_alignment(input)?;
    let justify = input
        .try_parse(parse_content_alignment_res)
        .unwrap_or(align);
    Some(PlaceContentShorthand { align, justify })
}

// ─────────────────────────────────────────────────────────────────────────
// CSS Grid Layout Module Level 1 parsers.
// ─────────────────────────────────────────────────────────────────────────

/// `<custom-ident>` 除外リスト — [`GridLineValue`] 系 production と
/// `<line-names>` (§7.2.2) の両方で使う ([`is_reserved_custom_ident`] に
/// `span` / `auto` を追加除外)。
///
/// CSS Grid Layout Module Level 1 §8.3 verbatim: "In all the above
/// productions, the `<custom-ident>` additionally excludes the keywords
/// `span` and `auto`"、および §7.2.2 verbatim: "A line name cannot be span
/// or auto, i.e. the `<custom-ident>` in the `<line-names>` production
/// excludes the keywords span and auto." — §7.2.2 自身がこの除外を明示的に
/// `<line-names>` production に適用すると述べている ([`is_reserved_counter_name`]
/// が base list に `none` を足す precedent と同じ pattern)。
pub(crate) fn is_reserved_grid_line_name(ident: &str) -> bool {
    is_reserved_custom_ident(ident)
        || matches!(ident.to_ascii_lowercase().as_str(), "span" | "auto")
}

/// [`GridLineValue`] 系 production 内の `<custom-ident>` を parse する
/// ([`is_reserved_grid_line_name`] の追加除外を適用する点のみ
/// [`parse_custom_ident`] と異なる)。
fn parse_grid_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_grid_line_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// [`parse_grid_custom_ident`] の `Result` 版 ([`parse_padding_side_res`] と
/// 同じ wrapper pattern、`&&`/`||` combinator 内の `try_parse` 用)。
fn parse_grid_custom_ident_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SmolStr, ParseError<'i, ()>> {
    parse_grid_custom_ident(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<line-names> = '[' <custom-ident>* ']'` (CSS Grid Layout Module Level 1
/// §7.2.2 "Naming Grid Lines: the `[<custom-ident>*]` syntax"
/// <https://www.w3.org/TR/css-grid-1/#named-lines>) を parse する。
///
/// `<line-names>` の `<custom-ident>` は [`is_reserved_grid_line_name`] の
/// 追加除外 (`span`/`auto`) を**受ける** — §7.2.2 verbatim: "A line name
/// cannot be span or auto, i.e. the `<custom-ident>` in the `<line-names>`
/// production excludes the keywords span and auto." (この除外は §8.3 の
/// `<grid-line>` production 群だけでなく、§7.2.2 自身が明示する)。
///
/// `[` が見つからなければ `None` (呼び出し元は
/// [`parse_line_names_or_empty`] 経由で空 `Vec` へ fallback)。空 `[]` は
/// `Some(vec![])`。
fn parse_line_names(input: &mut Parser<'_, '_>) -> Option<Vec<SmolStr>> {
    input
        .try_parse(|i| -> Result<Vec<SmolStr>, ParseError<'_, ()>> {
            i.expect_square_bracket_block()?;
            i.parse_nested_block(|inner| {
                let mut names = Vec::new();
                while !inner.is_exhausted() {
                    names.push(
                        parse_grid_custom_ident(inner).ok_or_else(|| inner.new_custom_error(()))?,
                    );
                }
                Ok(names)
            })
        })
        .ok()
}

/// [`GridTrackList::line_names`] / [`GridTrackRepeat::line_names`] の各
/// interleave slot を埋める helper — `<line-names>?` (省略可) を空 `Vec` に
/// 正規化する。
fn parse_line_names_or_empty(input: &mut Parser<'_, '_>) -> Vec<SmolStr> {
    parse_line_names(input).unwrap_or_default()
}

/// `<flex [0,∞]>` — the `fr` unit (CSS Grid Layout Module Level 1 §7.2.4
/// "Flexible Lengths: the fr unit"
/// <https://www.w3.org/TR/css-grid-1/#fr-unit>) を parse する。
///
/// non-negative **と** finite を parse 時に enforce する —
/// [`parse_nonneg_finite_number`] doc と同じ理由 (raikiri-dom bridge が
/// taffy `MaxTrackSizingFunction::fr` へ無変換で copy する、sink guard を
/// 経由しない経路)。
///
/// token 取得は `next_numeric_stable` 経由 (module doc 冒頭「Numeric-token
/// NaN stabilization」節参照) — zero-mantissa/huge-exponent literal
/// (`grid-template-columns: 0e999fr`) を cssparser tokenizer が `NaN` に
/// collapse する artifact をここで訂正済のため、spec-correct な `Flex(0.0)`
/// に解決される。
fn parse_grid_flex_res<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    match &next_numeric_stable(input)? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("fr") => {
            if value.is_finite() && *value >= 0.0 {
                Ok(*value)
            } else {
                Err(input.new_custom_error(()))
            }
        }
        t => Err(input.new_unexpected_token_error(t.clone())),
    }
}

/// `<track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content
/// | max-content | auto` を parse する (CSS Grid Layout Module Level 1 §7.2.1
/// "Track Sizes"
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-track-breadth>)。
///
/// `<length-percentage>` branch は必ず最後に試す —
/// [`parse_length_value`] は失敗時も token を consume する ([`parse_flex_basis`]
/// doc の同 rationale 参照) ため、それ以降に別 alternative を試せない。
fn parse_track_breadth(input: &mut Parser<'_, '_>) -> Option<GridTrackBreadth> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridTrackBreadth::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(GridTrackBreadth::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(GridTrackBreadth::MaxContent);
    }
    if let Ok(flex) = input.try_parse(parse_grid_flex_res) {
        return Some(GridTrackBreadth::Flex(flex));
    }
    let length = input
        .try_parse(|i| parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(())))
        .ok()?;
    (length_payload(length) >= 0.0).then_some(GridTrackBreadth::Length(length))
}

/// `<inflexible-breadth> = <length-percentage [0,∞]> | min-content |
/// max-content | auto` を parse する (CSS Grid Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#valdef-grid-template-columns-inflexible-breadth>)。
/// [`parse_track_breadth`] と同型だが `<flex>` branch を持たない。
fn parse_inflexible_breadth(input: &mut Parser<'_, '_>) -> Option<GridInflexibleBreadth> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridInflexibleBreadth::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("min-content"))
        .is_ok()
    {
        return Some(GridInflexibleBreadth::MinContent);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("max-content"))
        .is_ok()
    {
        return Some(GridInflexibleBreadth::MaxContent);
    }
    let length = input
        .try_parse(|i| parse_length_value(i, true).ok_or_else(|| i.new_custom_error::<(), ()>(())))
        .ok()?;
    (length_payload(length) >= 0.0).then_some(GridInflexibleBreadth::Length(length))
}

/// `minmax( <inflexible-breadth>, <track-breadth> )` を parse する (CSS Grid
/// Layout Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-minmax>)。
fn parse_grid_minmax_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(GridInflexibleBreadth, GridTrackBreadth), ParseError<'i, ()>> {
    input.expect_function_matching("minmax")?;
    input.parse_nested_block(|inner| {
        let min = parse_inflexible_breadth(inner).ok_or_else(|| inner.new_custom_error(()))?;
        inner.expect_comma()?;
        let max = parse_track_breadth(inner).ok_or_else(|| inner.new_custom_error(()))?;
        Ok((min, max))
    })
}

/// `fit-content( <length-percentage [0,∞]> )` を parse する (CSS Grid Layout
/// Module Level 1 §7.2.1
/// <https://www.w3.org/TR/css-grid-1/#funcdef-grid-template-columns-fit-content>)。
fn parse_grid_fit_content_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    input.expect_function_matching("fit-content")?;
    input.parse_nested_block(|inner| {
        let len = parse_length_value(inner, true).ok_or_else(|| inner.new_custom_error(()))?;
        if length_payload(len) >= 0.0 {
            Ok(len)
        } else {
            Err(inner.new_custom_error(()))
        }
    })
}

/// `<track-size>` を parse する ([`GridTrackSize`] doc 参照)。3 alternative
/// を順に試す (`minmax()` → `fit-content()` → bare `<track-breadth>`) —
/// 前 2 つは固有 function name で判別できるため順序に意味は薄いが、
/// bare `<track-breadth>` は必ず最後 ([`parse_track_breadth`] doc の
/// consume-on-failure 注記参照)。
fn parse_track_size(input: &mut Parser<'_, '_>) -> Option<GridTrackSize> {
    if let Ok((min, max)) = input.try_parse(parse_grid_minmax_res) {
        return Some(GridTrackSize::MinMax(min, max));
    }
    if let Ok(limit) = input.try_parse(parse_grid_fit_content_res) {
        return Some(GridTrackSize::FitContent(limit));
    }
    parse_track_breadth(input).map(GridTrackSize::Breadth)
}

/// [`GridTrackBreadth`] が `<fixed-breadth>` (= `<length-percentage>`のみ)
/// かどうか — [`grid_track_size_is_fixed`] の component。
fn grid_track_breadth_is_fixed(b: &GridTrackBreadth) -> bool {
    matches!(b, GridTrackBreadth::Length(_))
}

/// [`GridInflexibleBreadth`] が `<fixed-breadth>` かどうか —
/// [`grid_track_size_is_fixed`] の component。
fn grid_inflexible_breadth_is_fixed(b: &GridInflexibleBreadth) -> bool {
    matches!(b, GridInflexibleBreadth::Length(_))
}

/// [`GridTrackSize`] が `<fixed-size>` 制約 (CSS Grid Layout Module Level 1
/// §7.2.1 `<fixed-size> = <fixed-breadth> | minmax( <fixed-breadth>,
/// <track-breadth> ) | minmax( <inflexible-breadth>, <fixed-breadth> )`) を
/// 満たすかどうか — [`GridTrackSize`] doc の "fixed-size 制約" 節参照。
///
/// `minmax()` は min/max のどちらか一方が `<fixed-breadth>` であれば足りる
/// (spec 上両方が fixed である必要はない — `minmax(100px, 1fr)` は valid
/// `<fixed-size>`)。`fit-content()` は `<fixed-size>` の grammar に
/// alternative が無いため常に `false`。
pub(crate) fn grid_track_size_is_fixed(t: &GridTrackSize) -> bool {
    match t {
        GridTrackSize::Breadth(b) => grid_track_breadth_is_fixed(b),
        GridTrackSize::MinMax(min, max) => {
            grid_inflexible_breadth_is_fixed(min) || grid_track_breadth_is_fixed(max)
        }
        GridTrackSize::FitContent(_) => false,
    }
}

/// `repeat()` の repetition count (`<integer [1,∞]>` / `auto-fill` /
/// `auto-fit`) を parse する ([`GridRepeatCount`] doc 参照)。
fn parse_grid_repeat_count(input: &mut Parser<'_, '_>) -> Option<GridRepeatCount> {
    if input
        .try_parse(|i| i.expect_ident_matching("auto-fill"))
        .is_ok()
    {
        return Some(GridRepeatCount::AutoFill);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("auto-fit"))
        .is_ok()
    {
        return Some(GridRepeatCount::AutoFit);
    }
    let n = input.try_parse(|i| i.expect_integer()).ok()?;
    (n >= 1).then_some(GridRepeatCount::Count(n as u32))
}

/// `repeat( <count>, [ <line-names>? <track-size> ]+ <line-names>? )` を
/// parse する ([`GridTrackRepeat`] doc 参照)。count が
/// [`GridRepeatCount::Count`] か [`GridRepeatCount::AutoFill`]/
/// [`GridRepeatCount::AutoFit`] かで inner track が `<track-size>` /
/// `<fixed-size>` のどちらの grammar に従うべきかが変わる
/// ([`GridTrackRepeat`] doc の "許可される count と `<fixed-size>` 制約"
/// 節参照) が、本関数は grammar 上共通の shape (full `<track-size>`) で
/// parse し、`<fixed-size>` 制約は [`parse_grid_template_tracks`] が
/// track list 全体を見て post-validate する。
fn parse_grid_repeat_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GridTrackRepeat, ParseError<'i, ()>> {
    input.expect_function_matching("repeat")?;
    input.parse_nested_block(|inner| {
        let count = parse_grid_repeat_count(inner).ok_or_else(|| inner.new_custom_error(()))?;
        inner.expect_comma()?;
        let mut line_names = vec![parse_line_names_or_empty(inner)];
        let mut tracks = Vec::new();
        while let Some(size) = parse_track_size(inner) {
            tracks.push(size);
            line_names.push(parse_line_names_or_empty(inner));
        }
        if tracks.is_empty() {
            return Err(inner.new_custom_error(()));
        }
        Ok(GridTrackRepeat {
            count,
            line_names,
            tracks,
        })
    })
}

/// [`parse_grid_repeat_res`] の `Option` 版 —
/// [`parse_grid_track_list`] の alternation 用。
pub(crate) fn parse_grid_repeat(input: &mut Parser<'_, '_>) -> Option<GridTrackRepeat> {
    input.try_parse(parse_grid_repeat_res).ok()
}

/// `<track-list>` / `<auto-track-list>` の component 列 (`[ <line-names>?
/// [ <track-size> | <track-repeat> ] ]+ <line-names>?`) を parse する。
/// `<fixed-size>` 制約の検査は行わない ([`grid_track_list_obeys_auto_repeat_constraint`]
/// が呼び出し元 [`parse_grid_template_tracks`] で担う)。
fn parse_grid_track_list(input: &mut Parser<'_, '_>) -> Option<GridTrackList> {
    let mut line_names = vec![parse_line_names_or_empty(input)];
    let mut components = Vec::new();
    loop {
        if let Some(repeat) = parse_grid_repeat(input) {
            components.push(GridTrackListComponent::Repeat(repeat));
        } else if let Some(size) = parse_track_size(input) {
            components.push(GridTrackListComponent::Size(size));
        } else {
            break;
        }
        line_names.push(parse_line_names_or_empty(input));
    }
    if components.is_empty() {
        None
    } else {
        Some(GridTrackList {
            line_names,
            components,
        })
    }
}

/// [`parse_grid_track_list`] が返した [`GridTrackList`] が spec の 2 制約を
/// 満たすかどうかを検査する (post-parse validation、
/// [`GridTrackRepeat`] doc の "許可される count と `<fixed-size>` 制約" 節参照):
///
/// 1. CSS Grid Layout Module Level 1 §7.2.3.1 verbatim: "It can only appear
///    once in the track list" — auto-fill/auto-fit repeat は track list 中
///    に高々 1 つ。
/// 2. §7.2.3.1 verbatim: "Automatic repetitions (auto-fill or auto-fit)
///    cannot be combined with fully intrinsic or flexible sizes" —
///    auto-repeat が存在する場合、track list 中の**他の全 track**
///    (bare track と他の repeat() の中身の両方、auto-repeat 自身の中身も
///    含む) が [`grid_track_size_is_fixed`] を満たす必要がある。
fn grid_track_list_obeys_auto_repeat_constraint(list: &GridTrackList) -> bool {
    let auto_repeat_count = list
        .components
        .iter()
        .filter(|c| {
            matches!(
                c,
                GridTrackListComponent::Repeat(r)
                    if matches!(r.count, GridRepeatCount::AutoFill | GridRepeatCount::AutoFit)
            )
        })
        .count();
    if auto_repeat_count > 1 {
        return false;
    }
    if auto_repeat_count == 0 {
        return true;
    }
    list.components.iter().all(|c| match c {
        GridTrackListComponent::Size(size) => grid_track_size_is_fixed(size),
        GridTrackListComponent::Repeat(r) => r.tracks.iter().all(grid_track_size_is_fixed),
    })
}

/// `grid-template-columns` / `grid-template-rows`: `none | <track-list> |
/// <auto-track-list>` を parse する (CSS Grid Layout Module Level 1 §7.2
/// <https://www.w3.org/TR/css-grid-1/#track-sizing>、[`GridTemplateTracks`]
/// doc 参照)。
fn parse_grid_area_shorthand(input: &mut Parser<'_, '_>) -> Option<GridAreaShorthand> {
    let row_start = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let column_start = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let row_end = parse_grid_line(input)?;
    input.try_parse(|i| i.expect_delim('/')).ok()?;
    let column_end = parse_grid_line(input)?;
    Some(GridAreaShorthand {
        row_start,
        column_start,
        row_end,
        column_end,
    })
}

fn parse_grid_shorthand(input: &mut Parser<'_, '_>) -> Option<GridShorthand> {
    let rows = parse_grid_template_tracks(input)?;
    if input.try_parse(|i| i.expect_delim('/')).is_err() {
        return None;
    }
    let columns = parse_grid_template_tracks(input)?;
    Some(GridShorthand { rows, columns })
}

pub(crate) fn parse_grid_template_tracks(input: &mut Parser<'_, '_>) -> Option<GridTemplateTracks> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(GridTemplateTracks::None);
    }
    let list = parse_grid_track_list(input)?;
    if !grid_track_list_obeys_auto_repeat_constraint(&list) {
        return None;
    }
    Some(GridTemplateTracks::List(Arc::new(list)))
}

/// `grid-auto-columns` / `grid-auto-rows`: `<track-size>+` を parse する
/// (CSS Grid Layout Module Level 1 §7.6
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-columns>)。
/// `<track-list>` と異なり `repeat()` も `<line-names>` interleaving も
/// grammar に含まれない — bare `<track-size>` の並びのみ。
fn parse_grid_auto_track_list(input: &mut Parser<'_, '_>) -> Option<Arc<Vec<GridTrackSize>>> {
    let mut tracks = Vec::new();
    while let Some(size) = parse_track_size(input) {
        tracks.push(size);
    }
    if tracks.is_empty() {
        None
    } else {
        Some(Arc::new(tracks))
    }
}

/// `grid-auto-flow: [ row | column ] || dense` を parse する (CSS Grid
/// Layout Module Level 1 §7.7
/// <https://www.w3.org/TR/css-grid-1/#propdef-grid-auto-flow>)。
///
/// `dense` 単独 (axis 省略) は [`GridAutoFlowValue::RowDense`] に畳む —
/// axis 省略時の default が `row` であるため ([`GridAutoFlowValue`] doc の
/// 同型注記参照)。両 group とも順序自由 (`||`) なので最大 2 回のループで
/// 両方を試す。
fn parse_grid_auto_flow(input: &mut Parser<'_, '_>) -> Option<GridAutoFlowValue> {
    #[derive(Clone, Copy)]
    enum Axis {
        Row,
        Column,
    }
    let mut axis: Option<Axis> = None;
    let mut dense = false;
    for _ in 0..2 {
        if axis.is_none() && input.try_parse(|i| i.expect_ident_matching("row")).is_ok() {
            axis = Some(Axis::Row);
            continue;
        }
        if axis.is_none()
            && input
                .try_parse(|i| i.expect_ident_matching("column"))
                .is_ok()
        {
            axis = Some(Axis::Column);
            continue;
        }
        if !dense
            && input
                .try_parse(|i| i.expect_ident_matching("dense"))
                .is_ok()
        {
            dense = true;
            continue;
        }
        break;
    }
    match (axis, dense) {
        (None, false) => None,
        (None, true) => Some(GridAutoFlowValue::RowDense),
        (Some(Axis::Row), false) => Some(GridAutoFlowValue::Row),
        (Some(Axis::Row), true) => Some(GridAutoFlowValue::RowDense),
        (Some(Axis::Column), false) => Some(GridAutoFlowValue::Column),
        (Some(Axis::Column), true) => Some(GridAutoFlowValue::ColumnDense),
    }
}

/// `span <integer [1,∞]> || <custom-ident>` (`span` ident は呼び出し元が
/// 既に consume 済み) を parse する — [`parse_grid_line`] の tail helper。
fn parse_grid_line_span_tail(input: &mut Parser<'_, '_>) -> Option<GridLineValue> {
    let mut number: Option<u32> = None;
    let mut name: Option<SmolStr> = None;
    for _ in 0..2 {
        if number.is_none()
            && let Ok(n) = input.try_parse(|i| i.expect_integer())
        {
            if n < 1 {
                return None;
            }
            number = Some(n as u32);
            continue;
        }
        if name.is_none()
            && let Ok(n) = input.try_parse(parse_grid_custom_ident_res)
        {
            name = Some(n);
            continue;
        }
        break;
    }
    match (number, name) {
        (Some(n), Some(name)) => Some(GridLineValue::SpanNamed(name, n)),
        (Some(n), None) => Some(GridLineValue::Span(n)),
        (None, Some(name)) => Some(GridLineValue::SpanNamed(name, 1)),
        // `span` の直後に何も続かない — grammar 上 `span &&
        // [ <integer> || <custom-ident> ]` の右辺 group が必須のため invalid。
        (None, None) => None,
    }
}

/// `<grid-line>` を parse する ([`GridLineValue`] doc 参照)。
///
/// `[ [ <integer> ] && <custom-ident>? ]` alternative は order-free
/// (`&&`) なので、`<integer>` を最大 2 回のループで先に/後に両方試す —
/// **`<integer>` が一度も現れなければ** (`number.is_none()`)、それは
/// この alternative ではなく別の top-level alternative `<custom-ident>`
/// (単独) にマッチしたことを意味し、[`GridLineValue::Named`]
/// (bare-ident、shorthand omission-copy 規則の対象) を返す —
/// [`GridLineValue::NamedLine`] (`<integer>` 併記、対象外) とは
/// [`GridLineShorthand`] doc の spec verbatim 引用が要求する区別
/// ([`parse_grid_line_shorthand`] 参照)。
pub(crate) fn parse_grid_line(input: &mut Parser<'_, '_>) -> Option<GridLineValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(GridLineValue::Auto);
    }
    if input.try_parse(|i| i.expect_ident_matching("span")).is_ok() {
        return parse_grid_line_span_tail(input);
    }
    let mut number: Option<i32> = None;
    let mut name: Option<SmolStr> = None;
    for _ in 0..2 {
        if number.is_none()
            && let Ok(n) = input.try_parse(|i| i.expect_integer())
        {
            number = Some(n);
            continue;
        }
        if name.is_none()
            && let Ok(n) = input.try_parse(parse_grid_custom_ident_res)
        {
            name = Some(n);
            continue;
        }
        break;
    }
    match (number, name) {
        // `0` は spec verbatim "Negative integers or zero are invalid" —
        // named の有無に関わらず reject。
        (Some(0), _) => None,
        (Some(n), Some(name)) => Some(GridLineValue::NamedLine(name, n)),
        (Some(n), None) => Some(GridLineValue::Line(n)),
        (None, Some(name)) => Some(GridLineValue::Named(name)),
        (None, None) => None,
    }
}

/// `grid-row` / `grid-column`: `<grid-line> [ / <grid-line> ]?` shorthand を
/// parse する ([`GridLineShorthand`] doc 参照)。
pub(crate) fn parse_grid_line_shorthand(input: &mut Parser<'_, '_>) -> Option<GridLineShorthand> {
    let start = parse_grid_line(input)?;
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let end = parse_grid_line(input)?;
        return Some(GridLineShorthand { start, end });
    }
    // spec 本文 verbatim (`GridLineShorthand` doc 引用): 第 2 成分省略時、
    // 第 1 成分が `<custom-ident>` (= `GridLineValue::Named`、`<integer>`
    // 併記なしの bare 形のみ) なら第 2 成分にもその名前を copy、それ以外は
    // `auto`。
    let end = match &start {
        GridLineValue::Named(name) => GridLineValue::Named(name.clone()),
        _ => GridLineValue::Auto,
    };
    Some(GridLineShorthand { start, end })
}

/// `grid-template-areas` の 1 `<string>` を cell token 列へ分解する。
///
/// CSS Grid Layout Module Level 1 §7.3 verbatim tokenization 規則
/// (<https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>):
/// "Tokenize the string into a list of the following tokens, using
/// longest-match semantics": ident code point の並び (named cell) / `.` の
/// 並び (null cell) / whitespace (無視、トークン化されない) / それ以外
/// (trash token → invalid)。
///
/// `None` を返すのは trash token を検出した場合のみ (spec verbatim: "A
/// trash token is a syntax error, and makes the declaration invalid.")。
///
/// whitespace 判定は `char::is_whitespace()` (Unicode `White_Space`
/// property、`U+3000` 等の非 ASCII whitespace も含む) ではなく CSS Syntax 3
/// の whitespace 定義 (<https://www.w3.org/TR/css-syntax-3/#whitespace>)
/// verbatim: "A newline, U+0009 CHARACTER TABULATION, or U+0020 SPACE" を
/// 直接使う — newline は同 spec の input preprocessing
/// (<https://www.w3.org/TR/css-syntax-3/#input-preprocessing>) で
/// U+000D/U+000C が U+000A に正規化された後の定義なので、ここでは `'\n'`
/// のみを見ればよい (CR/FF は stylesheet 全体の tokenize 前処理で
/// 消えている前提)。
fn tokenize_grid_area_row(s: &str) -> Option<Vec<Option<SmolStr>>> {
    let mut cells = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if matches!(c, '\t' | '\n' | ' ') {
            chars.next();
        } else if c == '.' {
            while chars.peek() == Some(&'.') {
                chars.next();
            }
            cells.push(None);
        } else if is_grid_area_ident_char(c) {
            let mut name = String::new();
            while let Some(&c2) = chars.peek() {
                if !is_grid_area_ident_char(c2) {
                    break;
                }
                name.push(c2);
                chars.next();
            }
            cells.push(Some(SmolStr::new(name)));
        } else {
            return None;
        }
    }
    Some(cells)
}

/// CSS Syntax 3 の "ident code point" (letter / digit / `-` / `_` /
/// non-ASCII) を近似する classifier — [`tokenize_grid_area_row`] の
/// named-cell token 分解専用。escape sequence (`\XX`) は考慮しない —
/// `<string>` token の value は cssparser が既に unescape した literal
/// character 列であり、`grid-template-areas` の string tokenization
/// (§7.3) 自体は再度 CSS syntax としての escape 解釈を行わない (spec の
/// 定義がそのまま code point 単位の分類であるため)。
fn is_grid_area_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()
}

/// [`tokenize_grid_area_row`] 済みの row 列から [`GridTemplateAreas`] を
/// 構築する — named area の bounding box を算出し、spec §7.3 verbatim の
/// "If a named grid area spans multiple grid cells, but those cells do not
/// form a single filled-in rectangle, the declaration is invalid." を検査
/// する。
///
/// `rows` は呼び出し元 ([`parse_grid_template_areas`]) が非空を保証する
/// (空なら呼び出し元が先に `None` を返す)。
fn build_grid_template_areas(
    rows: Vec<Vec<Option<SmolStr>>>,
    row_strings: Vec<SmolStr>,
) -> Option<GridTemplateAreas> {
    let row_count = rows.len();
    let column_count = rows[0].len();
    // spec 本文 verbatim: "All strings must define the same number of cell
    // tokens ... and at least one cell token, or else the declaration is
    // invalid."
    if column_count == 0 || rows.iter().any(|r| r.len() != column_count) {
        return None;
    }
    // (name, row_min, row_max, col_min, col_max) — 初出順、線形 scan
    // (area 名の種類数は現実的に小さいため HashMap を持ち込まない)。
    let mut bounds: Vec<(SmolStr, usize, usize, usize, usize)> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some(name) = cell else { continue };
            match bounds.iter_mut().find(|(n, ..)| n == name) {
                Some((_, r0, r1, c0, c1)) => {
                    *r0 = (*r0).min(r);
                    *r1 = (*r1).max(r);
                    *c0 = (*c0).min(c);
                    *c1 = (*c1).max(c);
                }
                None => bounds.push((name.clone(), r, r, c, c)),
            }
        }
    }
    let mut areas = Vec::with_capacity(bounds.len());
    for (name, r0, r1, c0, c1) in bounds {
        let is_filled_rectangle = rows[r0..=r1]
            .iter()
            .all(|row| row[c0..=c1].iter().all(|cell| cell.as_ref() == Some(&name)));
        if !is_filled_rectangle {
            return None;
        }
        areas.push(GridTemplateAreaEntry {
            name,
            row_start: r0 as u32 + 1,
            row_end: r1 as u32 + 2,
            column_start: c0 as u32 + 1,
            column_end: c1 as u32 + 2,
        });
    }
    Some(GridTemplateAreas {
        row_strings,
        areas,
        row_count: row_count as u32,
        column_count: column_count as u32,
    })
}

/// `grid-template-areas: none | <string>+` を parse する (CSS Grid Layout
/// Module Level 1 §7.3
/// <https://www.w3.org/TR/css-grid-1/#grid-template-areas-property>、
/// [`GridTemplateAreasValue`] doc 参照)。
pub(crate) fn parse_grid_template_areas(
    input: &mut Parser<'_, '_>,
) -> Option<GridTemplateAreasValue> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(GridTemplateAreasValue::None);
    }
    let mut row_strings: Vec<SmolStr> = Vec::new();
    let mut rows: Vec<Vec<Option<SmolStr>>> = Vec::new();
    while let Ok(s) = input.try_parse(|i| i.expect_string().map(|s| SmolStr::new(s.as_ref()))) {
        let tokens = tokenize_grid_area_row(&s)?;
        row_strings.push(s);
        rows.push(tokens);
    }
    if rows.is_empty() {
        return None;
    }
    build_grid_template_areas(rows, row_strings)
        .map(|areas| GridTemplateAreasValue::Areas(Arc::new(areas)))
}

/// `place-items: <'align-items'> <'justify-items'>?` shorthand を parse
/// する (CSS Box Alignment Module Level 3 §7.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-items>)。第 2 成分
/// 省略時は spec 本文通り第 1 成分をそのまま copy する
/// ([`PlaceItemsShorthand`] doc 参照)。
pub(crate) fn parse_place_items_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<PlaceItemsShorthand> {
    let align = parse_self_alignment(input)?;
    let justify = input.try_parse(parse_self_alignment_res).unwrap_or(align);
    Some(PlaceItemsShorthand { align, justify })
}

/// `place-self: <'align-self'> <'justify-self'>?` shorthand を parse する
/// (CSS Box Alignment Module Level 3 §6.3
/// <https://www.w3.org/TR/css-align-3/#propdef-place-self>)。第 2 成分
/// 省略時の copy 規則は [`parse_place_items_shorthand`] と同じ。
pub(crate) fn parse_place_self_shorthand(input: &mut Parser<'_, '_>) -> Option<PlaceSelfShorthand> {
    let align = parse_align_self(input)?;
    let justify = input.try_parse(parse_align_self_res).unwrap_or(align);
    Some(PlaceSelfShorthand { align, justify })
}

/// `font-weight: <font-weight-absolute> | bolder | lighter` を parse する。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop>:
///
/// ```text
/// <font-weight-absolute> = [ normal | bold | <number [1,1000]> ]
/// ```
///
/// - `normal` = 400 / `bold` = 700 (spec §2.2 の keyword 定義)
/// - `bolder` / `lighter` は継承値依存の relative weight。parse 段では解けない
///   ため sentinel variant ([`FontWeightValue::Bolder`] /
///   [`FontWeightValue::Lighter`]) で保持し、[`crate::cascade::apply_value`]
///   が親の computed weight から解決する。
///
/// ASCII case-insensitive matching は CSS Values 3 §3.1 "Pre-defined Keywords"
/// <https://www.w3.org/TR/css-values-3/#keywords> 準拠 (sibling
/// `parse_display` / `parse_content_*` と同 convention)。
///
/// # Range (spec grammar)
///
/// spec §2.2: "Only values greater than or equal to 1, and less than or equal
/// to 1000, are valid, and all other values are invalid"。したがって `0` /
/// `1001` / `-100` の reject は **spec grammar そのもの** であり、stricter
/// policy ではない。範囲判定は **丸める前の指定値** に対して行う (spec の
/// "values" は author が書いた `<number>` を指すため、`0.6` や `1000.4` は
/// 丸めれば範囲内になるが invalid)。
///
/// # Fractional weight は丸めずそのまま保持する
///
/// **spec は fraction を落としてよいとは述べていない。** §2.2 の property table
/// は `Computed value: a number, see below` と規定し、§2.2.2 "Missing weights"
/// <https://www.w3.org/TR/css-fonts-4/#missing-weights> は "Fractional weights
/// are valid" と明言する。WPT `css/css-fonts/parsing/font-weight-computed.html`
/// の `test_computed_value('font-weight', '150.25')` (2-arg 形 = computed ==
/// specified) がこれを直接 check している。
///
/// payload ([`FontWeightValue::Absolute`]) と
/// [`crate::computed::ComputedValues::font_weight`] は共に `f32` (以前は
/// `u16` だったが格上げ) なので、parse 時に整数化する必要が
/// ない — `<number>` の `value` をそのまま保持する。旧実装は computed side が
/// `u16` だったため round-half-away-from-zero で整数化しており、その丸めが
/// §2.2.1 "Relative Weights" relative-weight table の*行選択*を変える 2 次被害
/// があった (親 `font-weight: 349.5` + 子 `bolder` が旧実装では 350 への丸め後
/// `350 <= w < 550` 行 → 700 に化け、spec の `100 <= w < 350` 行 → 400
/// と食い違う。`549.5` + `bolder`、`749.5` + `lighter` も同型 — check:
/// `crate::cascade::tests::bolder_lighter_resolve_against_unrounded_fractional_parent_weight`)。
/// `f32` 格上げにより丸めそのものが不要になったため、この 2 次被害も解消される。
pub(crate) fn parse_font_weight(input: &mut Parser<'_, '_>) -> Option<FontWeightValue> {
    match &next_numeric_stable(input).ok()? {
        // `<number [1,1000]>`。`value` field (f32) を見るので `1e3` のような
        // scientific notation や fractional もそのまま受理される (どちらも
        // CSS Values 3 の `<number>` production として spec-valid)。fraction は
        // 丸めずそのまま computed value まで運ぶ (上記 doc 参照)。
        // token 取得は `next_numeric_stable` 経由 (module doc「Numeric-token
        // NaN stabilization」節参照) — zero-mantissa/huge-exponent
        // (`0e999`) と huge-mantissa/underflowing-exponent (例:
        // `50` に等しい `5` + 400 zeros + `e-399`) の両方の cssparser
        // tokenizer artifact をここで訂正済のため、この範囲判定が実際に
        // `NaN` を見ることはもう無い。±inf (`1e400` 等、真正の magnitude
        // overflow) は依然どちらか片方の比較が false になり reject される
        // (§2.2 "all other values are invalid" と一致)。
        Token::Number { value, .. } if *value >= 1.0 && *value <= 1000.0 => {
            Some(FontWeightValue::Absolute(*value))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("normal") => {
            Some(FontWeightValue::Absolute(400.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bold") => {
            Some(FontWeightValue::Absolute(700.0))
        }
        Token::Ident(name) if name.eq_ignore_ascii_case("bolder") => Some(FontWeightValue::Bolder),
        Token::Ident(name) if name.eq_ignore_ascii_case("lighter") => {
            Some(FontWeightValue::Lighter)
        }
        _ => None,
    }
}

/// `font-style: <ident>` を parse する (CSS Fonts 4 §2.4
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>)。
///
/// Value grammar (§2.4, full property grammar): `normal | italic | left |
/// right | oblique <angle [-90deg,90deg]>?`。本 parser は `normal` /
/// `italic` / bare `oblique` の 3 keyword のみ受理する ([`FontStyle`] doc の
/// Scope carving 節参照) — `oblique` に続く `<angle>` 引数と `left` / `right`
/// は spec-valid だが未実装のため、他の未知 ident と同じく silent drop =
/// `None` とする。`oblique <angle>` (例: `oblique 14deg`) はこの関数自体は
/// `oblique` の ident だけを consume して成功で返るが、後続の `<angle>`
/// token が未消費のまま残るため、宣言全体が caller ([`mod@crate::rule`] の
/// `DeclParser`) の exhaustive-consumption check で drop される
/// ([`parse_text_transform`] doc の「case keyword が先」ケースと同 mechanism)。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_direction`]
/// と同 flavor)。
fn parse_font_style(input: &mut Parser<'_, '_>) -> Option<FontStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(FontStyle::Normal),
        "italic" => Some(FontStyle::Italic),
        "oblique" => Some(FontStyle::Oblique),
        _ => None,
    }
}

/// `font-variant-caps: <ident>` を parse する (CSS Fonts Module Level 3 §6.6
/// <https://www.w3.org/TR/css-fonts-3/#font-variant-caps-prop>)。
///
/// Value grammar (§6.6, full property grammar): `normal | small-caps |
/// all-small-caps | petite-caps | all-petite-caps | unicase |
/// titling-caps`。本 parser はこの 7 keyword 全てを受理する
/// ([`FontVariantCaps`] doc の「7 keyword の意味」節参照)。それ以外の ident は
/// spec-invalid = 未知 ident として silent drop = `None` とする。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_font_style`]
/// と同 flavor)。
fn parse_font_variant_caps(input: &mut Parser<'_, '_>) -> Option<FontVariantCaps> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(FontVariantCaps::Normal),
        "small-caps" => Some(FontVariantCaps::SmallCaps),
        "all-small-caps" => Some(FontVariantCaps::AllSmallCaps),
        "petite-caps" => Some(FontVariantCaps::PetiteCaps),
        "all-petite-caps" => Some(FontVariantCaps::AllPetiteCaps),
        "unicase" => Some(FontVariantCaps::Unicase),
        "titling-caps" => Some(FontVariantCaps::TitlingCaps),
        _ => None,
    }
}

/// Parse the CSS Text `text-transform` grammar.
///
/// The optional case keyword and width keywords may appear in any order. At
/// most one case keyword and each width keyword are accepted.
fn parse_text_transform(input: &mut Parser<'_, '_>) -> Option<TextTransform> {
    let mut case = None;
    let mut full_width = false;
    let mut full_size_kana = false;
    let mut count = 0;
    while count < 3 {
        let ident = match input.try_parse(|i| i.expect_ident().map(ToOwned::to_owned)) {
            Ok(ident) => ident,
            Err(_) => break,
        };
        count += 1;
        match ident.to_ascii_lowercase().as_str() {
            "none" if count == 1 => return Some(TextTransform::None),
            "capitalize" if case.is_none() => case = Some(TextTransform::Capitalize),
            "uppercase" if case.is_none() => case = Some(TextTransform::Uppercase),
            "lowercase" if case.is_none() => case = Some(TextTransform::Lowercase),
            "full-width" if !full_width => full_width = true,
            "full-size-kana" if !full_size_kana => full_size_kana = true,
            _ => return None,
        }
    }
    match (case, full_width, full_size_kana) {
        (None, false, false) => None,
        (None, true, false) => Some(TextTransform::FullWidth),
        (None, false, true) => Some(TextTransform::FullSizeKana),
        (None, true, true) => Some(TextTransform::FullWidthFullSizeKana),
        (Some(TextTransform::Capitalize), false, false) => Some(TextTransform::Capitalize),
        (Some(TextTransform::Uppercase), false, false) => Some(TextTransform::Uppercase),
        (Some(TextTransform::Lowercase), false, false) => Some(TextTransform::Lowercase),
        (Some(TextTransform::Capitalize), true, false) => Some(TextTransform::CapitalizeFullWidth),
        (Some(TextTransform::Uppercase), true, false) => Some(TextTransform::UppercaseFullWidth),
        (Some(TextTransform::Lowercase), true, false) => Some(TextTransform::LowercaseFullWidth),
        (Some(TextTransform::Capitalize), false, true) => {
            Some(TextTransform::CapitalizeFullSizeKana)
        }
        (Some(TextTransform::Uppercase), false, true) => Some(TextTransform::UppercaseFullSizeKana),
        (Some(TextTransform::Lowercase), false, true) => Some(TextTransform::LowercaseFullSizeKana),
        (Some(TextTransform::Capitalize), true, true) => {
            Some(TextTransform::CapitalizeFullWidthFullSizeKana)
        }
        (Some(TextTransform::Uppercase), true, true) => {
            Some(TextTransform::UppercaseFullWidthFullSizeKana)
        }
        (Some(TextTransform::Lowercase), true, true) => {
            Some(TextTransform::LowercaseFullWidthFullSizeKana)
        }
        _ => None,
    }
}

/// `visibility: <ident>` を parse する (CSS Display 3 §4
/// <https://www.w3.org/TR/css-display-3/#visibility>)。
///
/// Value grammar (spec verbatim): `visible | hidden | collapse`。全 3
/// keyword を受理する ([`Visibility`] doc の Scope carving 節参照 —
/// `collapse` の formatting-context 固有な space-saving 効果は未実装だが、
/// keyword 自体は spec-valid として受理する)。ASCII case-insensitive で
/// ident を比較する ([`parse_font_style`] と同 flavor)。
fn parse_visibility(input: &mut Parser<'_, '_>) -> Option<Visibility> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        "hidden" => Some(Visibility::Hidden),
        "collapse" => Some(Visibility::Collapse),
        _ => None,
    }
}

/// `word-break: <ident>` を parse する (CSS Text 3 §5.1
/// <https://www.w3.org/TR/css-text-3/#word-break-property>)。
///
/// Value grammar (§5.1, full property grammar): `normal | keep-all |
/// break-all | break-word`。本 parser は `normal` / `keep-all` /
/// `break-all` の 3 keyword のみ受理する ([`WordBreak`] doc の Scope
/// carving 節参照) — 4th keyword `break-word` (deprecated,
/// `word-break: normal` + `overflow-wrap: anywhere` の compound 相当) は
/// spec-valid だが未実装のため、他の未知 ident と同じく silent drop =
/// `None` とする。ASCII case-insensitive で ident を比較する (sibling
/// `parse_font_style` と同 flavor)。
fn parse_word_break(input: &mut Parser<'_, '_>) -> Option<WordBreak> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(WordBreak::Normal),
        "keep-all" => Some(WordBreak::KeepAll),
        "break-all" => Some(WordBreak::BreakAll),
        "manual" => Some(WordBreak::Manual),
        "auto-phrase" => Some(WordBreak::AutoPhrase),
        "break-word" => Some(WordBreak::BreakWord),
        _ => None,
    }
}

/// `overflow-wrap: <ident>` (`word-wrap` legacy alias 名でも呼ばれる、
/// [`OverflowWrap`] doc の「legacy alias」節参照) を parse する (CSS Text 3
/// §5.4 <https://www.w3.org/TR/css-text-3/#overflow-wrap-property>)。
///
/// Value grammar (§5.4): `normal | break-word | anywhere` — 3 keyword とも
/// 受理する (`WordBreak` の deprecated `break-word` とは異なり、
/// `overflow-wrap` 自身の `break-word` は deprecated ではない spec-valid
/// keyword、[`OverflowWrap`] doc 参照)。ASCII case-insensitive で ident を
/// 比較する (sibling `parse_word_break` と同 flavor)。
fn parse_overflow_wrap(input: &mut Parser<'_, '_>) -> Option<OverflowWrap> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

/// `float: <ident>` を parse する (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>, [`FloatValue`]
/// doc 参照)。
///
/// Value grammar: `left | right | none` (`inherit` は上記 "CSS-wide
/// keyword (canonical)" 節により未対応)。ASCII case-insensitive matching
/// は sibling `parse_word_break` と同 flavor。
fn parse_float(input: &mut Parser<'_, '_>) -> Option<FloatValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(FloatValue::None),
        "left" => Some(FloatValue::Left),
        "right" => Some(FloatValue::Right),
        "inline-start" => Some(FloatValue::InlineStart),
        "inline-end" => Some(FloatValue::InlineEnd),
        _ => None,
    }
}

/// `clear: <ident>` を parse する (CSS2 §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>, [`ClearValue`]
/// doc 参照)。
///
/// Value grammar: `none | left | right | both` (`inherit` は上記
/// "CSS-wide keyword (canonical)" 節により未対応)。ASCII case-insensitive
/// matching は sibling `parse_float` と同 flavor。
fn parse_clear(input: &mut Parser<'_, '_>) -> Option<ClearValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(ClearValue::None),
        "left" => Some(ClearValue::Left),
        "right" => Some(ClearValue::Right),
        "both" => Some(ClearValue::Both),
        "inline-start" => Some(ClearValue::InlineStart),
        "inline-end" => Some(ClearValue::InlineEnd),
        _ => None,
    }
}

/// `white-space: <ident>` を parse する (CSS Text 3 §3
/// <https://www.w3.org/TR/css-text-3/#white-space-property>)。
///
/// Value grammar (§3, full property grammar): `normal | pre | nowrap |
/// pre-wrap | break-spaces | pre-line`。本 parser は `normal` / `pre` /
/// `nowrap` / `pre-wrap` / `pre-line` の 5 keyword のみ受理する
/// ([`WhiteSpace`] doc の Scope carving 節参照) — 6th keyword
/// `break-spaces` は spec-valid だが未実装のため、他の未知 ident と同じく
/// silent drop = `None` とする。ASCII case-insensitive で ident を比較する
/// (sibling `parse_word_break` と同 flavor)。
fn parse_white_space(input: &mut Parser<'_, '_>) -> Option<WhiteSpace> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "pre" => Some(WhiteSpace::Pre),
        "nowrap" => Some(WhiteSpace::Nowrap),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        "break-spaces" => Some(WhiteSpace::BreakSpaces),
        _ => None,
    }
}

/// `hyphens: <ident>` を parse する (CSS Text 3 §5.3
/// <https://www.w3.org/TR/css-text-3/#hyphens-property>)。
///
/// Value grammar (§5.3): `none | manual | auto` — 3 keyword とも受理する。
/// `auto` は `manual` に collapse せず、parse 段では別 keyword として
/// そのまま [`Hyphens::Auto`] を返す ([`Hyphens`] doc の「Downstream
/// handoff」節 — 両者の扱いの一致は downstream consumer 側の実装判断であり、
/// この parser の責務ではない)。ASCII case-insensitive で ident を比較する
/// (sibling `parse_word_break` と同 flavor)。
/// Parses `text-wrap: wrap | nowrap` (subset, CSS Text 4 §5).
fn parse_text_wrap_mode(input: &mut Parser<'_, '_>) -> Option<TextWrapMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "wrap" => Some(TextWrapMode::Wrap),
        "nowrap" => Some(TextWrapMode::Nowrap),
        _ => None,
    }
}

fn parse_hyphens(input: &mut Parser<'_, '_>) -> Option<Hyphens> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(Hyphens::None),
        "manual" => Some(Hyphens::Manual),
        "auto" => Some(Hyphens::Auto),
        _ => None,
    }
}

fn parse_line_break(input: &mut Parser<'_, '_>) -> Option<LineBreak> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(LineBreak::Auto),
        "loose" => Some(LineBreak::Loose),
        "normal" => Some(LineBreak::Normal),
        "strict" => Some(LineBreak::Strict),
        "anywhere" => Some(LineBreak::Anywhere),
        _ => None,
    }
}

fn parse_text_justify(input: &mut Parser<'_, '_>) -> Option<TextJustify> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextJustify::Auto),
        "none" => Some(TextJustify::None),
        "inter-word" => Some(TextJustify::InterWord),
        "inter-character" => Some(TextJustify::InterCharacter),
        "distribute" => Some(TextJustify::Distribute),
        _ => None,
    }
}

fn parse_text_align_all(input: &mut Parser<'_, '_>) -> Option<TextAlignAll> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "start" => Some(TextAlignAll::Start),
        "end" => Some(TextAlignAll::End),
        "left" => Some(TextAlignAll::Left),
        "right" => Some(TextAlignAll::Right),
        "center" => Some(TextAlignAll::Center),
        "justify" => Some(TextAlignAll::Justify),
        "match-parent" => Some(TextAlignAll::MatchParent),
        _ => None,
    }
}

fn parse_text_align_last(input: &mut Parser<'_, '_>) -> Option<TextAlignLast> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextAlignLast::Auto),
        "start" => Some(TextAlignLast::Start),
        "end" => Some(TextAlignLast::End),
        "left" => Some(TextAlignLast::Left),
        "right" => Some(TextAlignLast::Right),
        "center" => Some(TextAlignLast::Center),
        "justify" => Some(TextAlignLast::Justify),
        "match-parent" => Some(TextAlignLast::MatchParent),
        _ => None,
    }
}

/// `display: <ident>` を parse する。
///
/// CSS Display 3 §2 "Box Layout Modes: the display property"
/// <https://www.w3.org/TR/css-display-3/#propdef-display>。現状受理する
/// keyword は 18 つ:
///
/// - `block` — `<display-outside>` (block flow)
/// - `inline` — `<display-outside>` (inline flow、initial value)
/// - `inline-block` — `<display-legacy>` (inline flow-root)
/// - `none` — `<display-box>` (subtree omitted from box tree)
/// - `flex` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block flex` と等価
/// - `grid` — `<display-inside>` (§2.2) keyword、outer-defaulting rule
///   により `block grid` と等価
/// - `list-item` — `<display-listitem>` keyword、outer-defaulting rule
///   により `block flow list-item` と等価。HTML Living Standard の default
///   UA stylesheet が `li` に指定する
///   (<https://html.spec.whatwg.org/multipage/rendering.html#lists>)。
///   [`DisplayValue::ListItem`] の doc が言う通り keyword acceptance のみ
///   — marker box 生成は本 crate scope 外。
/// - `contents` — `<display-box>` (§2.5)、要素自身が box を生成しない
///   ([`DisplayValue::Contents`] doc 参照)
/// - `table` — `<display-internal>` (block-level table wrapper)
/// - `inline-table` — `<display-internal>` (inline-level table wrapper)
/// - `table-row-group` — `<display-internal>` (`<tbody>`)
/// - `table-header-group` — `<display-internal>` (`<thead>`)
/// - `table-footer-group` — `<display-internal>` (`<tfoot>`)
/// - `table-row` — `<display-internal>` (`<tr>`)
/// - `table-column-group` — `<display-internal>` (`<colgroup>`)
/// - `table-column` — `<display-internal>` (`<col>`)
/// - `table-cell` — `<display-internal>` (`<td>`, `<th>`)
/// - `table-caption` — `<display-internal>` (`<caption>`)
///
/// `inline-flex` / `inline-grid` は現在の layout bridge ではそれぞれ
/// `flex` / `grid` と同じ formatting context として受理する (inline-level
/// shrink-to-fit の区別は未実装)。`flow-root` 等の他 keyword は未実装のため
/// silent drop (`None`)。ASCII case-insensitive で ident を比較する (CSS Values 3
/// §3.1 "Pre-defined Keywords" <https://www.w3.org/TR/css-values-3/#keywords>:
/// keyword は ASCII case-insensitive)。
fn parse_text_combine_upright(input: &mut Parser<'_, '_>) -> Option<TextCombineUpright> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(TextCombineUpright::None),
        "all" => Some(TextCombineUpright::All),
        _ => None,
    }
}

fn parse_text_orientation(input: &mut Parser<'_, '_>) -> Option<TextOrientation> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "mixed" => Some(TextOrientation::Mixed),
        "upright" => Some(TextOrientation::Upright),
        "sideways" => Some(TextOrientation::Sideways),
        _ => None,
    }
}

fn parse_unicode_bidi(input: &mut Parser<'_, '_>) -> Option<UnicodeBidi> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(UnicodeBidi::Normal),
        "embed" => Some(UnicodeBidi::Embed),
        "isolate" => Some(UnicodeBidi::Isolate),
        "bidi-override" => Some(UnicodeBidi::BidiOverride),
        "isolate-override" => Some(UnicodeBidi::IsolateOverride),
        "plaintext" => Some(UnicodeBidi::Plaintext),
        _ => None,
    }
}

/// `table-layout: <ident>` を parse する (CSS Tables 3 §4
/// <https://www.w3.org/TR/css-tables-3/#table-layout-property>,
/// [`TableLayoutValue`] doc 参照)。
///
/// Value grammar: `auto | fixed`。ASCII case-insensitive matching は
/// sibling [`parse_unicode_bidi`] と同 flavor、余剰 token
/// (`table-layout: auto fixed` 等) は caller (`rule.rs::DeclParser`) の
/// `expect_exhausted` が drop する。
fn parse_table_layout(input: &mut Parser<'_, '_>) -> Option<TableLayoutValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TableLayoutValue::Auto),
        "fixed" => Some(TableLayoutValue::Fixed),
        _ => None,
    }
}

/// `border-collapse: <ident>` を parse する (CSS Tables 3 §6
/// <https://www.w3.org/TR/css-tables-3/#border-collapse-property>,
/// [`BorderCollapseValue`] doc 参照)。
///
/// Value grammar: `collapse | separate`。matching 規則は sibling
/// [`parse_table_layout`] と同じ。
fn parse_border_collapse(input: &mut Parser<'_, '_>) -> Option<BorderCollapseValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "collapse" => Some(BorderCollapseValue::Collapse),
        "separate" => Some(BorderCollapseValue::Separate),
        _ => None,
    }
}

/// `border-spacing: <length>{1,2}` を parse する (CSS Tables 3 §6.1
/// <https://www.w3.org/TR/css-tables-3/#border-spacing-property>,
/// [`BorderSpacingValue`] doc 参照)。
///
/// 各成分は [`parse_non_negative_length`] (`<length [0,∞]>`、
/// `<percentage>` alternative なし — spec の Percentages: N/A と
/// "Negative lengths are illegal" を共に enforce)。unitless `0` は
/// [`parse_length_value`] の CSS Values 3 §5 unitless-zero clause で
/// `Px(0.0)` として受理される (WPT computed の `"0"` → `"0px"` case)。
/// 第 2 成分省略時は第 1 成分を copy する (spec 本文 +
/// [`GapShorthand`] と同型)。3 成分以上・bare non-zero number・`%` は
/// caller (`rule.rs::DeclParser`) の `expect_exhausted` / 各成分の `None`
/// で drop する。
fn parse_border_spacing(input: &mut Parser<'_, '_>) -> Option<BorderSpacingValue> {
    let horizontal = parse_non_negative_length(input)?;
    let vertical = input
        .try_parse(|i| parse_non_negative_length(i).ok_or(()))
        .unwrap_or(horizontal);
    Some(BorderSpacingValue {
        horizontal,
        vertical,
    })
}

/// `caption-side: <ident>` を parse する (CSS Tables 3 §7
/// <https://www.w3.org/TR/css-tables-3/#caption-side-property>,
/// [`CaptionSideValue`] doc 参照)。
///
/// Value grammar: `top | bottom`。ASCII case-insensitive matching は
/// sibling [`parse_table_layout`] と同 flavor、余剰 token
/// (`caption-side: top bottom` 等) は caller (`rule.rs::DeclParser`) の
/// `expect_exhausted` が drop する。
fn parse_caption_side(input: &mut Parser<'_, '_>) -> Option<CaptionSideValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "top" => Some(CaptionSideValue::Top),
        "bottom" => Some(CaptionSideValue::Bottom),
        _ => None,
    }
}

/// `empty-cells: <ident>` を parse する (CSS Tables 3 §8
/// <https://www.w3.org/TR/css-tables-3/#empty-cells-property>,
/// [`EmptyCellsValue`] doc 参照)。
///
/// Value grammar: `show | hide`。matching 規則は sibling
/// [`parse_caption_side`] と同じ。
fn parse_empty_cells(input: &mut Parser<'_, '_>) -> Option<EmptyCellsValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "show" => Some(EmptyCellsValue::Show),
        "hide" => Some(EmptyCellsValue::Hide),
        _ => None,
    }
}

fn parse_display(input: &mut Parser<'_, '_>) -> Option<DisplayValue> {
    // sibling multi-keyword idiom (parse_string_fetch / parse_content_part /
    // parse_content_text_keyword) に揃える。ASCII case-insensitive matching は
    // to_ascii_lowercase() 経由 (parse-time allocation は一 declaration 一回)。
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "block" => Some(DisplayValue::Block),
        "inline" => Some(DisplayValue::Inline),
        "inline-block" => Some(DisplayValue::InlineBlock),
        "flow-root" => Some(DisplayValue::FlowRoot),
        "none" => Some(DisplayValue::None),
        // The current layout bridge models the outer display type as block,
        // so inline-flex/inline-grid share the corresponding formatting
        // context until inline-level shrink-to-fit support is added.
        "flex" => Some(DisplayValue::Flex),
        "inline-flex" => Some(DisplayValue::InlineFlex),
        "grid" => Some(DisplayValue::Grid),
        "inline-grid" => Some(DisplayValue::InlineGrid),
        "list-item" => Some(DisplayValue::ListItem),
        "contents" => Some(DisplayValue::Contents),
        "table" => Some(DisplayValue::Table),
        "inline-table" => Some(DisplayValue::InlineTable),
        "table-row-group" => Some(DisplayValue::TableRowGroup),
        "table-header-group" => Some(DisplayValue::TableHeaderGroup),
        "table-footer-group" => Some(DisplayValue::TableFooterGroup),
        "table-row" => Some(DisplayValue::TableRow),
        "table-column-group" => Some(DisplayValue::TableColumnGroup),
        "table-column" => Some(DisplayValue::TableColumn),
        "table-cell" => Some(DisplayValue::TableCell),
        "table-caption" => Some(DisplayValue::TableCaption),
        _ => None,
    }
}

/// Parse `list-style-type`'s `<counter-style-name> | <string>` grammar.
///
/// The built-in names are intentionally not enumerated here: CSS Lists 3
/// permits author-defined counter styles, so an otherwise valid identifier is
/// preserved for the marker resolver. Reserved CSS-wide keywords are rejected
/// as values rather than accidentally becoming custom counter-style names.
fn parse_list_style_type(input: &mut Parser<'_, '_>) -> Option<ListStyleType> {
    if let Ok(value) = input.try_parse(|i| i.expect_string_cloned()) {
        return Some(ListStyleType::String(SmolStr::new(value.as_ref())));
    }
    let ident = parse_custom_ident(input)?;
    match ident.to_ascii_lowercase().as_str() {
        "disc" => Some(ListStyleType::Disc),
        "none" => Some(ListStyleType::None),
        _ => Some(ListStyleType::Named(ident)),
    }
}

/// Parse `list-style-position: inside | outside`.
fn parse_list_style_position(input: &mut Parser<'_, '_>) -> Option<ListStylePosition> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "inside" => Some(ListStylePosition::Inside),
        "outside" => Some(ListStylePosition::Outside),
        _ => None,
    }
}

/// `box-sizing: <ident>` を parse する
/// (CSS Sizing 3 §3.3 <https://www.w3.org/TR/css-sizing-3/#box-sizing>)。
///
/// Spec value grammar (§3.3): `content-box | border-box`。ASCII
/// case-insensitive で ident を比較する (CSS Values 3 §3.1 "Pre-defined
/// Keywords"、sibling [`parse_display`] / [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`BoxSizing`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 他 keyword (`padding-box` — CSS-UI 3 draft 相当
///   だが css-sizing-3 では削除、`margin-box` 等) は silent drop = `None`。
fn parse_box_sizing(input: &mut Parser<'_, '_>) -> Option<BoxSizing> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

/// `text-align: <ident>` を parse する
/// (CSS Text 3 §6.1 <https://www.w3.org/TR/css-text-3/#text-align-property>)。
///
/// Spec value grammar (§6.1): `start | end | left | right | center | justify |
/// match-parent | justify-all`。ASCII case-insensitive で ident を比較する
/// (CSS spec 慣行、sibling [`parse_string_fetch`] / [`parse_content_part`] /
/// [`parse_content_text_keyword`] と同 flavor)。
///
/// # Scope carving ([`TextAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `<string>` value は silent drop。CSS Text 3
///   §6.1 の grammar には無く、CSS Text 4 §7.1
///   <https://www.w3.org/TR/css-text-4/#text-align-property> で追加された
///   alternative (semantics は同 §7.2 "Character-based Alignment in a Table
///   Column")。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 未知 keyword (`middle` 等) は silent drop = `None`。
fn parse_text_align(input: &mut Parser<'_, '_>) -> Option<TextAlign> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "start" => Some(TextAlign::Start),
        "end" => Some(TextAlign::End),
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        "justify" => Some(TextAlign::Justify),
        "match-parent" => Some(TextAlign::MatchParent),
        "justify-all" => Some(TextAlign::JustifyAll),
        _ => None,
    }
}

/// `direction: <ident>` を parse する
/// (CSS Writing Modes 4 §2.1 <https://www.w3.org/TR/css-writing-modes-4/#direction>)。
///
/// Spec value grammar (§2.1): `ltr | rtl`。ASCII case-insensitive で ident を
/// 比較する (sibling [`parse_text_align`] と同 flavor)。
///
/// # Scope carving ([`Direction`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: `ltr` / `rtl` 以外の ident は silent drop = `None`。
fn parse_direction(input: &mut Parser<'_, '_>) -> Option<Direction> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "ltr" => Some(Direction::Ltr),
        "rtl" => Some(Direction::Rtl),
        _ => None,
    }
}

/// `writing-mode: <ident>` を parse する
/// (CSS Writing Modes 4 §3.2 <https://www.w3.org/TR/css-writing-modes-4/#propdef-writing-mode>)。
///
/// Spec value grammar (§3.2): `horizontal-tb | vertical-rl | vertical-lr |
/// sideways-rl | sideways-lr`。ASCII case-insensitive で ident を比較する
/// (sibling [`parse_direction`] と同 flavor)。5 keyword とも spec 通り
/// 受理する — `vertical-rl` 以降 4 keyword の computed value normalization は
/// 本関数の責務ではなく [`resolve_writing_mode`] が担う ([`WritingMode`] doc の
/// Scope carving 節参照)。
///
/// # Scope carving ([`WritingMode`] doc-comment に詳述)
///
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
fn parse_writing_mode(input: &mut Parser<'_, '_>) -> Option<WritingMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "horizontal-tb" => Some(WritingMode::HorizontalTb),
        "vertical-rl" => Some(WritingMode::VerticalRl),
        "vertical-lr" => Some(WritingMode::VerticalLr),
        "sideways-rl" => Some(WritingMode::SidewaysRl),
        "sideways-lr" => Some(WritingMode::SidewaysLr),
        _ => None,
    }
}

/// `overflow-x` / `overflow-y: <ident>` を parse する (
/// CSS Overflow 3 §3.1 <https://www.w3.org/TR/css-overflow-3/#overflow-properties>)。
///
/// Spec value grammar (§3.1): `visible | hidden | clip | scroll | auto`。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_box_sizing`] /
/// [`parse_direction`] と同 flavor)。
///
/// # Scope carving ([`OverflowValue`] doc-comment に詳述)
///
/// - **(a) spec-invalid**: 上記 5 keyword 以外の ident は silent drop = `None`。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
fn parse_overflow_value(input: &mut Parser<'_, '_>) -> Option<OverflowValue> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "visible" => Some(OverflowValue::Visible),
        "hidden" => Some(OverflowValue::Hidden),
        "clip" => Some(OverflowValue::Clip),
        "scroll" => Some(OverflowValue::Scroll),
        "auto" => Some(OverflowValue::Auto),
        _ => None,
    }
}

/// `overflow: <'overflow-block'>{1,2}` shorthand — 1-2 value expansion。
///
/// grammar reference: CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>。
///
/// # Expansion rule (spec verbatim, §3.1)
///
/// "The overflow property is a shorthand property that sets the specified
/// values of overflow-x and overflow-y in that order. If the second value is
/// omitted, it is copied from the first."
///
/// [`parse_margin_shorthand`] と同じ try_parse 積み上げ pattern の 2-value
/// 版 (1-4 value ではなく 1-2 value であること以外は同型)。
///
/// # Trailing garbage handling
///
/// 3rd value (`overflow: hidden scroll auto`) は本 helper では 2 value 消費
/// して残り 1 token を unconsumed で return する。caller の
/// [`mod@crate::rule`] の `DeclParser` の
/// [`cssparser::DeclarationParser::parse_value`] impl が `expect_exhausted`
/// で余剰 token を検知して declaration ごと drop する
/// ([`parse_margin_shorthand`] doc の「Trailing garbage handling」節と同じ
/// 責務分担)。
fn parse_overflow_shorthand(input: &mut Parser<'_, '_>) -> Option<OverflowXY> {
    let v1 = parse_overflow_value(input)?;
    // 2nd value 不在 → 1 value case: 両 axis に spread (§3.1 "If the second
    // value is omitted, it is copied from the first.")。
    let Some(v2) = input.try_parse(|i| parse_overflow_value(i).ok_or(())).ok() else {
        return Some(OverflowXY::both(v1));
    };
    Some(OverflowXY { x: v1, y: v2 })
}

/// `text-decoration-line: none | [ underline || overline || line-through ||
/// blink ]` を parse する (CSS Text Decoration Module Level 3 §2.1
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-line-property>)。
///
/// # top-level alternative (`none` vs. `||` combination)
///
/// grammar は `none | [ ... ]` — `none` は他 4 keyword と併記不可能な
/// **別 alternative** (`none underline` は spec-invalid) であり、`none` 自体が
/// `||` combination の一員ではない。よって `none` を最初に単独で試し、
/// 一致すれば即 return する。
///
/// # `||` (any-order, each-at-most-once) loop
///
/// `none` に一致しなければ、[`parse_border_shorthand`] の per-slot
/// `try_parse` loop と同じ shape で 4 keyword を順不同・重複無しに peel する
/// (詳細な rationale は同関数 doc 参照)。4 keyword の ident 集合は互いに
/// disjoint (border shorthand の width/style/color 3 slot が disjoint なのと
/// 同じ理由 — 単純に別々の語)。
///
/// - unfilled flag (未 true の bool field) のみ試行
/// - 埋まっている flag に対する 2 回目の同一 keyword は、その flag の
///   `try_parse` を試さない (falls through) ので match せず loop を抜ける —
///   caller ([`mod@crate::rule`] の `DeclParser`) の `expect_exhausted` が
///   leftover token を検知して declaration ごと drop する
///   (`text-decoration-line: underline underline` は 0 decl になる)
/// - 4 flag とも埋まった、またはどの keyword にも match しなくなったら break
/// - 1 個も flag が立たなければ (`none` でもなく、`||` combination も 0 個)
///   `None` — spec `||` grammar の "one or more of them must occur" 違反
pub(crate) fn parse_text_decoration_line(input: &mut Parser<'_, '_>) -> Option<TextDecorationLine> {
    // top-level alternative: `none`。`||` combination とは併記不可 (上記 doc)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationLine::NONE);
    }
    // top-level alternative: `spelling-error` / `grammar-error`。互いに
    // 排他、かつ `||` group とも併記不可 — 単独 ident の場合のみ受理し、
    // 後続 token が残れば caller の `expect_exhausted` が落とす
    // (spec grammar `none | [ ... ] | spelling-error | grammar-error`)。
    if input
        .try_parse(|i| i.expect_ident_matching("spelling-error"))
        .is_ok()
    {
        return Some(TextDecorationLine::SPELLING_ERROR);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("grammar-error"))
        .is_ok()
    {
        return Some(TextDecorationLine::GRAMMAR_ERROR);
    }

    let mut line = TextDecorationLine::NONE;
    loop {
        if !line.underline
            && input
                .try_parse(|i| i.expect_ident_matching("underline"))
                .is_ok()
        {
            line.underline = true;
            continue;
        }
        if !line.overline
            && input
                .try_parse(|i| i.expect_ident_matching("overline"))
                .is_ok()
        {
            line.overline = true;
            continue;
        }
        if !line.line_through
            && input
                .try_parse(|i| i.expect_ident_matching("line-through"))
                .is_ok()
        {
            line.line_through = true;
            continue;
        }
        if !line.blink
            && input
                .try_parse(|i| i.expect_ident_matching("blink"))
                .is_ok()
        {
            line.blink = true;
            continue;
        }
        break;
    }

    if line == TextDecorationLine::NONE {
        // `none` は上で既に処理済み — ここに来るのは 0 keyword しか
        // match しなかった場合のみ (未知 ident、または value 自体が空)。
        return None;
    }
    Some(line)
}

/// `text-decoration-style: solid | double | dotted | dashed | wavy` を
/// parse する (CSS Text Decoration Module Level 3 §2.2
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-style-property>)。
/// ASCII case-insensitive で ident を比較する (sibling
/// [`parse_border_style_side`] と同 flavor)。
fn parse_text_decoration_style(input: &mut Parser<'_, '_>) -> Option<TextDecorationStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "solid" => Some(TextDecorationStyle::Solid),
        "double" => Some(TextDecorationStyle::Double),
        "dotted" => Some(TextDecorationStyle::Dotted),
        "dashed" => Some(TextDecorationStyle::Dashed),
        "wavy" => Some(TextDecorationStyle::Wavy),
        _ => None,
    }
}

/// `text-decoration-color: <color>` を parse する (CSS Text Decoration Module
/// Level 3 §2.3
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-color-property>)。
///
/// [`parse_border_color`] と同型 — `currentcolor` keyword (CSS Color 3 §4.4)
/// を先取りしてから [`parse_color`] (hex / named / `rgb(a)` / `transparent`)
/// に委譲する。独立した helper にしてあるのは、両 property が異なる
/// payload 型 ([`TextDecorationColor`] / [`BorderColor`]) を持つため —
/// [`parse_border_color`] 自体は border-*-color 専用のまま変更しない。
fn parse_text_decoration_color(input: &mut Parser<'_, '_>) -> Option<TextDecorationColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(TextDecorationColor::CurrentColor);
    }
    parse_color(input).map(TextDecorationColor::Resolved)
}

/// `text-decoration: <'text-decoration-line'> || <'text-decoration-style'> ||
/// <'text-decoration-color'>` shorthand を parse する (CSS Text Decoration
/// Module Level 3 §2.4
/// <https://www.w3.org/TR/css-text-decor-3/#text-decoration-property>)。
///
/// [`parse_border_shorthand`] と同じ 3-slot `||` loop (line / style / color)
/// — 詳細な rationale・loop 構造・initial value fill の判断根拠は同関数 doc
/// 参照。3 slot の ident/token 集合は互いに disjoint: line keyword
/// (`none`/`underline`/`overline`/`line-through`/`blink`) と style keyword
/// (`solid`/`double`/`dotted`/`dashed`/`wavy`) はどちらも named CSS color
/// ではなく ([`parse_named_color`] のテーブルに無い)、[`parse_color`] の
/// Ident 分岐に誤って吸われることはない。
///
/// # Initial value fill (省略成分)
///
/// spec §2.4 verbatim: "Omitted values are set to their initial values."
/// - line 省略 → [`TextDecorationLine::NONE`] (§2.1 initial)
/// - style 省略 → [`TextDecorationStyle::Solid`] (§2.2 initial)
/// - color 省略 → [`TextDecorationColor::CurrentColor`] (§2.3 initial)
pub(crate) fn parse_text_decoration_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationShorthand> {
    let mut line: Option<TextDecorationLine> = None;
    let mut style: Option<TextDecorationStyle> = None;
    let mut color: Option<TextDecorationColor> = None;
    let mut thickness: Option<TextDecorationThickness> = None;

    loop {
        if line.is_some() && style.is_some() && color.is_some() && thickness.is_some() {
            break;
        }
        if line.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationLine, ParseError<'_, ()>> {
                parse_text_decoration_line(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            line = Some(v);
            continue;
        }
        if style.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationStyle, ParseError<'_, ()>> {
                parse_text_decoration_style(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<TextDecorationColor, ParseError<'_, ()>> {
                parse_text_decoration_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }
        if thickness.is_none()
            && let Ok(v) =
                input.try_parse(|i| -> Result<TextDecorationThickness, ParseError<'_, ()>> {
                    parse_text_decoration_thickness(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            thickness = Some(v);
            continue;
        }
        break;
    }

    // spec `||` grammar: at least 1 component 必須。0 component は `None` =
    // declaration drop (`parse_border_shorthand` と同じ判断)。
    if line.is_none() && style.is_none() && color.is_none() && thickness.is_none() {
        return None;
    }

    Some(TextDecorationShorthand {
        line: line.unwrap_or(TextDecorationLine::NONE),
        style: style.unwrap_or(TextDecorationStyle::Solid),
        color: color.unwrap_or(TextDecorationColor::CurrentColor),
        thickness: thickness.unwrap_or(TextDecorationThickness::Auto),
    })
}

/// `text-decoration-skip-ink: auto | none | all` を parse する
/// (ED §2.10.4 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-ink-property>)。
/// ASCII case-insensitive で ident を比較する (sibling [`parse_text_decoration_style`]
/// と同 flavor)。
fn parse_text_decoration_skip_ink(input: &mut Parser<'_, '_>) -> Option<TextDecorationSkipInk> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(TextDecorationSkipInk::Auto),
        "none" => Some(TextDecorationSkipInk::None),
        "all" => Some(TextDecorationSkipInk::All),
        _ => None,
    }
}

/// `text-decoration-skip-spaces: none | all | [ start || end ]` を parse する
/// (ED §2.10.3 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-skip-spaces-property>)。
/// `none` / `all` は単独 (top-level alternative)、`start` / `end` は `||`
/// loop で各最大 1 回 ([`parse_text_decoration_line`] の 4-keyword loop と同形)。
fn parse_text_decoration_skip_spaces(
    input: &mut Parser<'_, '_>,
) -> Option<TextDecorationSkipSpaces> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(TextDecorationSkipSpaces::None);
    }
    if input.try_parse(|i| i.expect_ident_matching("all")).is_ok() {
        return Some(TextDecorationSkipSpaces::All);
    }
    let mut start = false;
    let mut end = false;
    loop {
        if !start
            && input
                .try_parse(|i| i.expect_ident_matching("start"))
                .is_ok()
        {
            start = true;
            continue;
        }
        if !end && input.try_parse(|i| i.expect_ident_matching("end")).is_ok() {
            end = true;
            continue;
        }
        break;
    }
    match (start, end) {
        (true, false) => Some(TextDecorationSkipSpaces::Start),
        (false, true) => Some(TextDecorationSkipSpaces::End),
        (true, true) => Some(TextDecorationSkipSpaces::StartEnd),
        (false, false) => None,
    }
}

/// `text-decoration-thickness: auto | from-font | <length-percentage>` を parse する
/// (ED §2.4.1 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-thickness-property>)。
/// `<line-width>` (`thin`/`medium`/`thick`) は scope 外のため drop
/// ([`TextDecorationThickness`] doc 参照)。`<length-percentage>` は
/// [`parse_length_value`] (`allow_percentage=true`) に委譲し、sign check
/// はしない (spec grammar に range 制限なし)。
fn parse_text_decoration_thickness(input: &mut Parser<'_, '_>) -> Option<TextDecorationThickness> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextDecorationThickness::Auto);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("from-font"))
        .is_ok()
    {
        return Some(TextDecorationThickness::FromFont);
    }
    parse_length_value(input, true).map(TextDecorationThickness::Length)
}

/// `text-decoration-inset: <length>{1,2} | auto` を parse する
/// (ED §2.9.1 <https://drafts.csswg.org/css-text-decor-4/#text-decoration-inset-property>)。
/// ED grammar は `<length-percentage>` だが WPT が `%` を reject するため
/// `allow_percentage=false` ([`TextDecorationInset`] doc 参照)。負値・
/// `calc()` は受理する (`calc` は [`parse_value`] の deferred 経路が
/// 事前に `DeferredValue` 化し、`純粋 [`Length`] のみ
/// ここに届く)。1 値目の場合は 2 値目を 1 値目に複製する
/// (margin/padding の 2-value 規則と同型)。
fn parse_text_decoration_inset(input: &mut Parser<'_, '_>) -> Option<TextDecorationInset> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextDecorationInset::Auto);
    }
    let start = parse_length_value(input, false)?;
    let end = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_length_value(i, false).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(start);
    Some(TextDecorationInset::Lengths { start, end })
}

/// `text-emphasis-position: auto | ([ over | under ] && [ right | left ]?)` を parse する
/// (ED §3.4 <https://drafts.csswg.org/css-text-decor-4/#text-emphasis-position-property>)。
/// `auto` 単独、それ以外は vertical 必須 (`over`/`under`) +
/// horizontal 任意 (`right`/`left`)、順序自由・各最大 1 回。
///
/// `auto` 以外の位置に `auto` が来たら leftover として caller が drop する
/// ([`parse_font_style`] の oblique-angle 取扱いと同 mechanism)。
fn parse_text_emphasis_position(input: &mut Parser<'_, '_>) -> Option<TextEmphasisPosition> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextEmphasisPosition::Auto);
    }
    let mut vertical: Option<TextEmphasisVEdge> = None;
    let mut horizontal: Option<TextEmphasisHEdge> = None;
    loop {
        if vertical.is_none() {
            if input.try_parse(|i| i.expect_ident_matching("over")).is_ok() {
                vertical = Some(TextEmphasisVEdge::Over);
                continue;
            }
            if input
                .try_parse(|i| i.expect_ident_matching("under"))
                .is_ok()
            {
                vertical = Some(TextEmphasisVEdge::Under);
                continue;
            }
        }
        if horizontal.is_none() {
            if input
                .try_parse(|i| i.expect_ident_matching("right"))
                .is_ok()
            {
                horizontal = Some(TextEmphasisHEdge::Right);
                continue;
            }
            if input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
                horizontal = Some(TextEmphasisHEdge::Left);
                continue;
            }
        }
        break;
    }
    Some(TextEmphasisPosition::Position {
        vertical: vertical?,
        horizontal,
    })
}

/// `text-underline-position: auto | [ from-font | under ] || [ left | right ]` を parse する
/// (ED §2.7 <https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property>)。
/// 3 slot (`from-font` / `under` / horizontal) の `||` loop +
/// post-check: `from-font` と `under` の併記は reject
/// (`under from-font` invalid)、`left`+`right` 併記も reject
/// (`left right` invalid、horizontal slot が 1 個のため
/// loop 段階で保証)、`auto` 単独 (leftover は caller が drop)。
fn parse_text_underline_position(input: &mut Parser<'_, '_>) -> Option<TextUnderlinePosition> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(TextUnderlinePosition::AUTO);
    }
    let mut pos = TextUnderlinePosition::AUTO;
    loop {
        if !pos.from_font
            && !pos.under
            && input
                .try_parse(|i| i.expect_ident_matching("from-font"))
                .is_ok()
        {
            pos.from_font = true;
            continue;
        }
        if !pos.under
            && !pos.from_font
            && input
                .try_parse(|i| i.expect_ident_matching("under"))
                .is_ok()
        {
            pos.under = true;
            continue;
        }
        if !pos.left && !pos.right && input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
            pos.left = true;
            continue;
        }
        if !pos.right
            && !pos.left
            && input
                .try_parse(|i| i.expect_ident_matching("right"))
                .is_ok()
        {
            pos.right = true;
            continue;
        }
        break;
    }
    if pos == TextUnderlinePosition::AUTO {
        return None;
    }
    Some(pos)
}

/// `vertical-align: <ident> | <length>` を parse する (CSS 2.1 §10.8.1
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>)。
///
/// Ident は ASCII case-insensitive で比較する (sibling [`parse_direction`]
/// / [`parse_text_decoration_style`] と同 flavor)。ident 側を先に
/// `try_parse` で試し、ident token でなければ (= dimension/number token
/// の可能性があれば) `<length>` として再挑戦する — [`parse_flex_basis`]
/// / [`parse_letter_or_word_spacing`] と同じ「keyword 群 → length
/// フォールバック」構造。
///
/// # Scope carving ([`VerticalAlign`] doc-comment に詳述)
///
/// - **(b) 非対応**: `top` / `bottom` keyword は silent drop = `None` —
///   [`VerticalAlign`] doc 参照 (inline formatting context 依存)。
/// - **(b) 非対応**: CSS-wide keyword は未実装 (将来対応)、silent drop
///   (5 keyword の一覧・理由は [`PropertyValue`] doc の「CSS-wide keyword」節
///   が canonical)。
/// - **(a) spec-invalid**: `baseline` / `sub` / `super` / `middle` /
///   `text-top` / `text-bottom` 以外の ident、および `<length>` /
///   `<percentage>` grammar に合わない token は silent drop = `None`。
/// - `<length>` / `<percentage>` に non-negative filter は掛けない — spec が
///   "Raise (positive value) or lower (negative value)" と明示的に負値を
///   許容する ([`parse_letter_or_word_spacing`] と同じ判断、
///   `padding`/`border-width` の non-negative constraint とは対照的)。
///   percentage の解決は [`crate::resolve::resolve_vertical_align`] が
///   `used_line_height_length` 基準で行い、`normal` 時は `0px` fallback
///   (同関数 doc 参照)。
fn parse_vertical_align(input: &mut Parser<'_, '_>) -> Option<VerticalAlign> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return match ident.to_ascii_lowercase().as_str() {
            "baseline" => Some(VerticalAlign::Baseline),
            "sub" => Some(VerticalAlign::Sub),
            "super" => Some(VerticalAlign::Super),
            "middle" => Some(VerticalAlign::Middle),
            "text-top" => Some(VerticalAlign::TextTop),
            "text-bottom" => Some(VerticalAlign::TextBottom),
            "top" => Some(VerticalAlign::Top),
            "bottom" => Some(VerticalAlign::Bottom),
            _ => None,
        };
    }
    parse_length_value(input, true).map(VerticalAlign::Length)
}

/// CSS Paged Media 3 §8.1 の `page` を parse する
/// (<https://www.w3.org/TR/css-page-3/#using-named-pages>)。
///
/// Grammar: `auto | <custom-ident>`。`auto` 単独、それ以外は CSS-wide keyword
/// (`inherit`/`initial`/`unset`/`revert`/`revert-layer`) と `default`
/// を除く単一 ident ([`PageValue`] doc 参照)。
/// ASCII case-insensitive で比較し、保持する値は
/// authored のまま (Atom は case-sensitive)。
fn parse_page_value(input: &mut Parser<'_, '_>) -> Option<PageValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(PageValue::Auto);
    }
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" => None,
        _ => Some(PageValue::Named(Atom::from(ident.as_ref()))),
    }
}

/// `counter-reset` / `counter-increment` / `counter-set` の value を parse する。
///
/// Grammar (CSS Lists 3 §4):
///   `<counter-name> = <custom-ident>` — CSS-wide keyword (inherit / initial /
///   unset / revert / revert-layer) + `default` + `none` を除く任意 ident。
///   `[ <counter-name> <integer>? ]+ | none`。
///
/// `default_number`: 各 property の spec default (reset=0、increment=1、set=0)。
///
/// `none` を top-level alternative として先に処理。以降は ident + optional
/// integer を LL(1) で peel。ident が reserved keyword、または最初の token が
/// ident でない (`counter-reset: 123 abc` 等) 場合は None を返し、rule.rs 側の
/// silent-drop で declaration が丸ごと落ちる。
///
/// 途中 ident (`chapter none`) が reserved の場合は `try_parse` の rewind で
/// 未消費のまま loop を抜け、caller の `expect_exhausted` (rule.rs)
/// が leftover token を検出して declaration を drop する。
fn parse_counter_property(
    input: &mut Parser<'_, '_>,
    default_number: i32,
) -> Option<Vec<(SmolStr, i32)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        // reserved keyword を counter-name として受理しない (spec §4、`<custom-ident>`
        // の除外リスト)。try_parse の rewind で reserved 検出時は unconsumed に戻す。
        let name = match input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = i.expect_ident()?.clone();
            if is_reserved_counter_name(&ident) {
                Err(i.new_custom_error(()))
            } else {
                Ok(SmolStr::new(ident.as_ref()))
            }
        }) {
            Ok(name) => name,
            Err(_) => break,
        };
        // optional trailing `<integer>` (missing → property-specific default)。
        let value = input
            .try_parse(|i| i.expect_integer())
            .unwrap_or(default_number);
        result.push((name, value));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `quotes` の value を parse する。
///
/// Grammar (legacy CSS2 §12.3.1 subset, [`PropertyValue::Quotes`] doc 参照):
///   `quotes = none | [ <string> <string> ]+`
///
/// `none` を top-level alternative として先に処理。以降は `<string>` を LL(1)
/// で 2 個ずつ pair にして peel する。
///
/// `parse_counter_property` (`<counter-name> <integer>?` — integer 省略時は
/// property-specific default で補える) と異なり、`<string> <string>` の pair
/// は片方が欠けた時点でその entry 自体が spec-invalid になる (grammar に
/// optional 要素が無い) — このため、1 個目の `<string>` を消費した後 2 個目が
/// 取れない (trailing unpaired `<string>`、またはその位置に `<string>` でない
/// token が来た) 場合は、途中まで蓄積した pair を捨てて即座に declaration
/// 全体を drop する (`None` を返す)。counter-* 系のように「ここまでの pair は
/// 残し、以降を caller の `expect_exhausted` (rule.rs) に委ねる」設計には
/// しない — 奇数個の `<string>` は `[ <string> <string> ]+` に決して一致しない
/// ため、削るべき「途中まで正しい prefix」自体が存在しない。
fn parse_quotes_property(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, SmolStr)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    loop {
        let open = match input.try_parse(|i| i.expect_string_cloned()) {
            Ok(s) => SmolStr::new(s.as_ref()),
            Err(_) => break,
        };
        let close = match input.try_parse(|i| i.expect_string_cloned()) {
            Ok(s) => SmolStr::new(s.as_ref()),
            // trailing unpaired `<string>` — spec-invalid, reject the whole
            // declaration (see function doc).
            Err(_) => return None,
        };
        result.push((open, close));
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// `<counter-name>` = `<custom-ident>` の除外リスト (CSS Lists 3 §4 + CSS Values 4
/// §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword + `default` (Counter Styles L3) + `none` (top-level alternative)
/// を弾く。case-insensitive 比較。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
pub(crate) fn is_reserved_counter_name(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default" | "none"
    )
}

/// `content: normal | none | <content-list>` を parse する
/// (CSS Content 3 §1 <https://www.w3.org/TR/css-content-3/#content-property>)。
///
/// `normal` / `none` は spec で意味が異なる (pseudo-element の生成/非生成)。
/// `normal` は初期値として空 `Vec` に留め、明示的な `none` は内部 sentinel
/// [`ContentComponent::None`] にして downstream の pseudo-element consumer が
/// marker を抑制できるようにする。
///
/// items+ loop は `<string>` literal と function token (`counter(...)` 等) を
/// 順次 peel する。認識できない token に当たった時点で loop を break、caller
/// の `expect_exhausted` (rule.rs) が leftover を検知して declaration ごと drop。
///
/// `alt text` (spec `... [/ <string>...]?`) は現状 scope 外、`/` 以降は
/// unconsumed のまま caller に返す (現状 rule.rs の `expect_exhausted` により
/// declaration drop、alt text 対応時に本関数を extend)。
pub(crate) fn parse_content(input: &mut Parser<'_, '_>) -> Option<Vec<ContentComponent>> {
    // `normal` / `none` = 空 list (top-level alternative)。
    if input
        .try_parse(|i| i.expect_ident_matching("normal"))
        .is_ok()
    {
        return Some(Vec::new());
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(vec![ContentComponent::None]);
    }

    let items = parse_content_list_items(input, ContentListMode::CssContent3);
    if items.is_empty() { None } else { Some(items) }
}

/// `<content-list>` の items+ loop 部分。
///
/// `<string>` bare literal、`<image>` の `<url>` alternative、bare keyword
/// (`contents` / `<quote>`)、function token (`counter(...)` / `string(...)` /
/// `target-*()` / `attr(...)` / `content(...)` / `leader(...)`) を順次 peel。
/// 認識できない token に当たった時点で break — 呼び出し側が leftover を検知
/// して drop する。
///
/// `content` property (`parse_content`) と `string-set` property
/// (`parse_string_set`) の両方から call されるが、GCPM 3 §1.1.1 は string-set
/// 向けに CSS Content 3 §2 の broad list を narrower に再定義しているため、
/// `mode` パラメータで受理 alternative 集合を分岐する:
/// - [`ContentListMode::CssContent3`] — content property (CSS Content 3 §2
///   <https://www.w3.org/TR/css-content-3/#content-values>)。10 alt full set
///   を受理 (`<image>` / `contents` / `<quote>` /
///   `leader()` は後に追加)。
/// - [`ContentListMode::GcpmStringSet`] — string-set property (CSS GCPM 3
///   §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#content-list>)。`string()` /
///   `target-counter()` / `target-counters()` / `target-text()` / `<image>` /
///   `contents` / `<quote>` / `leader()` は GCPM 3 §1.1.1 L82 narrow grammar
///   に含まれず reject (bare `<string>` literal は両 mode で受理)。
///
/// bare literal 分岐は spec 上両 mode で共通 (どちらの `<content-list>` grammar
/// も `<string>` を top-level alternative に含む) なので mode 判定なし。他の
/// 分岐は各 branch 内で mode guard を掛ける ([`parse_content_function`] の
/// match arm guard と同じ pattern)。`Parser::try_parse` は失敗時に読んだ token
/// を必ず rewind するため、branch の試行順序は正しさに影響しない
/// (どの順で並べても等価)。
///
/// (導入後、mode-parameterize を経て `<image>` / `contents` / `<quote>` /
/// `leader()` を追加)
fn parse_content_list_items(
    input: &mut Parser<'_, '_>,
    mode: ContentListMode,
) -> Vec<ContentComponent> {
    let mut items = Vec::new();
    loop {
        // bare `<string>` literal — 両 mode 共通 (mode gate 不要)。
        if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
            items.push(ContentComponent::Literal(SmolStr::new(s.as_ref())));
            continue;
        }
        // `<image>` の `<url>` alternative — CSS Images 3 <url> production
        // (`url(...)` / `url("...")`) のみ (`<gradient>` は未実装として
        // defer、`ContentComponent::Image` doc 参照)。`expect_url` は bare
        // quoted string を受理しない (`<url> = <url()> | <src()>`) ので上の
        // literal 分岐との誤 overlap は無い。CssContent3 mode 限定
        // (GCPM 3 §1.1.1 L82 narrow list に `<image>` は含まれない)。
        if mode == ContentListMode::CssContent3
            && let Ok(url) = input.try_parse(|i| i.expect_url())
        {
            items.push(ContentComponent::Image {
                url: url.as_ref().to_string(),
            });
            continue;
        }
        // bare keyword alternative — `contents` / `<quote>` (function でも
        // `<string>` でもない ident-only alternative)。CssContent3 mode 限定。
        if mode == ContentListMode::CssContent3
            && let Ok(c) = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                parse_content_bare_keyword(ident.as_ref()).ok_or_else(|| i.new_custom_error(()))
            })
        {
            items.push(c);
            continue;
        }
        // function — mode に応じて `string()` / `target-*()` / `leader()` を
        // reject する判定は `parse_content_function` の match arm side で実施。
        let parsed = input.try_parse(|i| -> Result<ContentComponent, ParseError<'_, ()>> {
            let name = i.expect_function()?.clone();
            i.parse_nested_block(|inner| {
                parse_content_function(name.as_ref(), mode, inner)
                    .ok_or_else(|| inner.new_custom_error(()))
            })
        });
        match parsed {
            Ok(c) => items.push(c),
            Err(_) => break,
        }
    }
    items
}

/// `contents` keyword と `<quote>` (`open-quote` / `close-quote` /
/// `no-open-quote` / `no-close-quote`) の bare-ident alternative をまとめて
/// 判定する ([`parse_content_list_items`] 専用 helper)。CSS Content 3 §2.3
/// <https://www.w3.org/TR/css-content-3/#element-content> および §2.4.2
/// <https://www.w3.org/TR/css-content-3/#quote-values>。
fn parse_content_bare_keyword(ident: &str) -> Option<ContentComponent> {
    match ident.to_ascii_lowercase().as_str() {
        "contents" => Some(ContentComponent::Contents),
        "open-quote" => Some(ContentComponent::Quote(QuoteKeyword::OpenQuote)),
        "close-quote" => Some(ContentComponent::Quote(QuoteKeyword::CloseQuote)),
        "no-open-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoOpenQuote)),
        "no-close-quote" => Some(ContentComponent::Quote(QuoteKeyword::NoCloseQuote)),
        _ => None,
    }
}

/// `string-set: none | [ <custom-ident> <content-list> ]#` を parse する
/// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
///
/// `none` を top-level alternative として先に処理し、以降は
/// `(name, content-list)` entry を comma-separated で peel する。
///
/// `<custom-ident>` は CSS-wide keyword + `default` (css-values-4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents> が将来の CSS-wide
/// keyword 用に予約) + `none` (top-level alt、gcpm-3 §1.1.1) を弾く。
///
/// ## Entry separator の strict 化
///
/// `#` (comma-separated multiplier、CSS Values 4 §2.3
/// <https://www.w3.org/TR/css-values-4/#mult-comma>) は entry 間に comma を
/// 要求する一方、**trailing comma を許容しない**。従って comma を consume した
/// 直後の loop iteration では次 entry の name parse **必須** — 失敗すれば
/// `#` production 全体が spec-invalid、declaration drop = `None`。
///
/// 初回 iteration で name parse が失敗する case (`string-set: ,`,
/// `string-set: "x"` 等 name 不在) も含めて `.ok()?` で一律に `None` 上位伝播
/// する。この strict `?` propagation は sibling
/// [`parse_optional_counter_style`] と同 principle。
///
/// `<content-list>` は 1+ items 必須 (CSS Content 3 §2)。name の後に 1 item も
/// peel できなければ malformed → `None` (declaration drop)。
fn parse_string_set(input: &mut Parser<'_, '_>) -> Option<Vec<(SmolStr, Vec<ContentComponent>)>> {
    // `none` = empty list (top-level alternative)。
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }

    let mut entries = Vec::new();
    loop {
        // <custom-ident> — CSS-wide keyword + `default` + `none` を弾く。
        // 既存 `is_reserved_custom_ident` (css-wide + default) と、property-specific
        // top-level alternative の `none` reject を組み合わせる (
        // `is_reserved_custom_ident` docstring の想定 usage)。
        //
        // `.ok()?` で strict 上位伝播: (a) 初回 iteration で name 不在 = `#`
        // production 0 entries、(b) 直前 iteration で bottom `expect_comma` が
        // succeed した直後 = trailing comma、の 2 case を一律 `None` に落とす。
        let name = input
            .try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
                let ident = i.expect_ident()?.clone();
                if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                    Err(i.new_custom_error(()))
                } else {
                    Ok(SmolStr::new(ident.as_ref()))
                }
            })
            .ok()?;
        // <content-list> は 1+ items 必須。0 items → declaration drop。
        // GCPM 3 §1.1.1 narrow local <content-list> = `string()` と `target-*()`
        // を受理しない (詳細は `ContentListMode` doc)。
        let items = parse_content_list_items(input, ContentListMode::GcpmStringSet);
        if items.is_empty() {
            return None;
        }
        entries.push((name, items));
        // 次 entry の separator: comma で継続、他 token で loop を抜ける
        // (caller `expect_exhausted` が leftover token を drop)。break 到達時は
        // 直前の push で entries 非空 — なので tail は無条件 `Some(entries)`。
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }

    // break 到達 = 直前の push を経ている、`.ok()?` 経路以外で loop を抜ける
    // 唯一の exit なので `entries` は必ず 1+。
    Some(entries)
}

/// Dispatch on function name (ASCII-case-insensitive、spec identifier 慣行)。
/// 未知の function name または引数 parse 失敗は `None` — caller の
/// `parse_nested_block` が custom error に変換する。
///
/// `mode` は property ごとの `<content-list>` 語彙を選ぶ (詳細は
/// [`ContentListMode`] doc):
/// - [`ContentListMode::CssContent3`] (`content` property, CSS Content 3 §2)
///   では全 arm を許可。
/// - [`ContentListMode::GcpmStringSet`] (`string-set` property, CSS GCPM 3
///   §1.1.1) では `string` / `target-counter` / `target-counters` /
///   `target-text` / `leader` arm を match guard で外し fall-through で `None`
///   を返す (= declaration drop、caller の `parse_string_set` が
///   `<content-list>` 0 items → `None`)。`counter` / `counters` / `content` /
///   `attr` は両 mode で spec grammar に含まれるため gate なし。
///
/// `<image>` (`url()`) / `contents` / `<quote>` は function 名 dispatch では
/// なく [`parse_content_list_items`] 側の bare-token branch で扱う (`<image>`
/// は url token、`contents`/`<quote>` は bare ident であり `expect_function`
/// にヒットしないため)。
///
/// 各 `parse_*_fn` は自身では `expect_exhausted` を呼ばない —
/// [`parse_content`] 側の `parse_nested_block` が内部で
/// [`Parser::parse_entirely`] を経由し、closure 成功後の余剰 token を
/// exhaustion check で拒否する ([`parse_rgb_function`] と同じ規約)。
fn parse_content_function(
    name: &str,
    mode: ContentListMode,
    input: &mut Parser<'_, '_>,
) -> Option<ContentComponent> {
    match name.to_ascii_lowercase().as_str() {
        "string" if matches!(mode, ContentListMode::CssContent3) => parse_string_fn(input),
        "counter" => parse_counter_fn(input),
        "counters" => parse_counters_fn(input),
        "attr" => parse_attr_fn(input),
        "target-counter" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counter_fn(input)
        }
        "target-counters" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_counters_fn(input)
        }
        "target-text" if matches!(mode, ContentListMode::CssContent3) => {
            parse_target_text_fn(input)
        }
        "content" => parse_content_fn(input),
        "leader" if matches!(mode, ContentListMode::CssContent3) => parse_leader_fn(input),
        _ => None,
    }
}

/// `<custom-ident>` (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>): CSS-wide keyword と
/// `default` を除いた任意 ident。case-preserving、smol str で保持。
///
/// `none` はここでは除外しない。spec verbatim: "Specifications using
/// `<custom-ident>` must specify clearly what other keywords are excluded
/// from `<custom-ident>`, if any…" と述べるとおり、より狭い grammar
/// (`<counter-name>` 等) の追加除外は個別の predicate (例
/// [`is_reserved_counter_name`]) 側の責務。[`is_reserved_custom_ident`] の
/// docstring も参照。
///
/// **呼び出し元は当初 3 箇所**: `string()` の name 引数 ([`parse_string_fn`])、
/// `target-counter()` / `target-counters()` の第 2 引数
/// ([`parse_target_counter_fn`] / [`parse_target_counters_fn`])。いずれも spec 上
/// `<custom-ident>` を取り `none` は valid。
///
/// 後に `pub(crate)` に広げ、`counter_style` module が
/// `<counter-style-name>` (CSS Counter Styles L3 §3
/// <https://www.w3.org/TR/css-counter-styles-3/#typedef-counter-style-name> —
/// `<custom-ident>` に `none` 追加除外を足した production、`<symbol>` の
/// `<custom-ident>` alternative 等) の base として同じ CSS-wide keyword 除外
/// list を再利用する 4 箇所目の呼び出し元になった (`is_reserved_custom_ident`
/// の list を二重管理しないため — 本 crate の drift 回避規約、
/// [`crate::page::PageCascadeResult::declarations`] doc 同旨)。
///
/// `<counter-name>` を取る `counter()` / `counters()` および counter-* property は
/// **本関数を経由しない** — [`parse_counter_name`] / [`parse_counter_property`] が
/// [`is_reserved_counter_name`] で `none` を追加除外する。したがって本関数に
/// `none` 除外を足してはならない (足すと `target-counter(url(#a), none)` と
/// `string(none)` を spec に反して reject する)。
pub(crate) fn parse_custom_ident(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_custom_ident(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `<custom-ident>` 除外リスト (CSS Values 4 §4.2
/// <https://www.w3.org/TR/css-values-4/#custom-idents>)。
///
/// CSS-wide keyword (`inherit` / `initial` / `unset` / `revert` /
/// `revert-layer`) と `default` のみを弾く。`none` はここでは除外せず、
/// より狭い grammar (`<counter-name>` 等) の追加除外は個別の predicate
/// (例 [`is_reserved_counter_name`]) で行う。case-insensitive 比較。
///
/// `pub(crate)`: `counter_style` module が
/// `<counter-style-name>` 系 production (rule name / `fallback` / `system:
/// extends`) の除外 predicate を組み立てる際にこの base list を再利用する
/// ([`parse_custom_ident`] の doc 参照)。
///
/// これは CSS Values 4 §4.2 の permanent な spec 除外規定であり、**CSS-wide
/// keyword の実装状況とは無関係** — [`PropertyValue`] doc の「CSS-wide keyword」節
/// が説明する「property value としては未実装」claim
/// とは別の話なので混同しないこと。
pub(crate) fn is_reserved_custom_ident(ident: &str) -> bool {
    matches!(
        ident.to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
    )
}

/// `string(<custom-ident> [, [ first | start | last | first-except ]? ])`。
/// CSS Content 3 §2.7.2 <https://www.w3.org/TR/css-content-3/#string-function>。
fn parse_string_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_custom_ident(input)?;
    let fetch = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_string_fetch(input)?
    } else {
        StringFetchMode::default()
    };
    Some(ContentComponent::String { name, fetch })
}

fn parse_string_fetch(input: &mut Parser<'_, '_>) -> Option<StringFetchMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "first" => Some(StringFetchMode::First),
        "start" => Some(StringFetchMode::Start),
        "last" => Some(StringFetchMode::Last),
        "first-except" => Some(StringFetchMode::FirstExcept),
        _ => None,
    }
}

/// `<counter-name>` (CSS Lists 3 §4
/// <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>):
/// `<custom-ident>` から `none` を追加除外した production。
/// spec verbatim: "A `<counter-name>` name cannot match the keyword `none`;
/// such an identifier is invalid as a `<counter-name>`"。
///
/// counter() / counters() (§4.7) の first argument、および
/// counter-reset / counter-increment / counter-set property
/// (§4.1 / §4.2) の name 引数で使う。後者は既に [`parse_counter_property`] が
/// [`is_reserved_counter_name`]
/// 経由で reject 済 — 本 helper は前者を同じ predicate に揃えるための wrapper。
fn parse_counter_name(input: &mut Parser<'_, '_>) -> Option<SmolStr> {
    let ident = input.expect_ident().ok()?.clone();
    if is_reserved_counter_name(&ident) {
        None
    } else {
        Some(SmolStr::new(ident.as_ref()))
    }
}

/// `counter(<counter-name>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counter { name, style })
}

/// `counters(<counter-name>, <string>, <counter-style>?)`。
/// CSS Lists 3 §4.7 <https://www.w3.org/TR/css-lists-3/#counter-functions>。
/// first argument grammar は §4 `<counter-name>`
/// (<https://www.w3.org/TR/css-lists-3/#typedef-counter-name>) —
/// `<custom-ident>` から `none` を追加除外。
fn parse_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = parse_counter_name(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::Counters {
        name,
        separator,
        style,
    })
}

/// optional trailing `, <counter-style>`。省略時は spec default `decimal`
/// (CSS Lists 3 §4.7 `counter()` / `counters()` の末尾引数
/// <https://www.w3.org/TR/css-lists-3/#counter-functions>、CSS Content 3 §2.6.1-2
/// `target-counter()` / `target-counters()` の末尾引数
/// <https://www.w3.org/TR/css-content-3/#target-counter>)。
///
/// grammar は `<counter-style>?` — `,` を先行させる時は ident 必須。
/// `,` を consume 後に ident 不在 (`counter(chapter,)` 等の trailing-comma)
/// は spec-invalid、`None` 上位伝播で declaration ごと drop する
/// (sibling [`parse_string_fetch`] / [`parse_content_part`] と同じ strict
/// `?` propagation、silent Decimal fallback は撤去済み)。
fn parse_optional_counter_style(input: &mut Parser<'_, '_>) -> Option<CounterStyle> {
    if input.try_parse(|i| i.expect_comma()).is_ok() {
        // comma consumed — ident 必須。失敗は None として上位伝播。
        let ident = input.expect_ident().ok()?.clone();
        Some(counter_style_from_ident(ident.as_ref()))
    } else {
        Some(CounterStyle::default())
    }
}

pub(crate) fn counter_style_from_ident(ident: &str) -> CounterStyle {
    if ident.eq_ignore_ascii_case("decimal") {
        CounterStyle::Decimal
    } else {
        CounterStyle::Named(SmolStr::new(ident))
    }
}

/// `attr(<attribute-name> [, <fallback>])`。CSS Content 3 §2.1 and
/// CSS Values and Units 5 §7.7.1.
///
/// This phase intentionally supports only untyped fallbacks that are either a
/// quoted string or a single identifier. The latter is retained as an
/// invalid-fallback marker: it contributes no text when the attribute is
/// absent, matching the measured `invalid` case without pretending to support
/// typed `attr()` syntax.
fn parse_attr_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let name = input.expect_ident().ok()?.clone();
    let name = SmolStr::new(name.as_ref());
    if input.try_parse(|i| i.expect_comma()).is_err() {
        return Some(ContentComponent::Attr { name });
    }

    let fallback = if let Ok(value) =
        input.try_parse(|i| i.expect_string().map(|value| SmolStr::new(value.as_ref())))
    {
        Some(value)
    } else {
        // A single non-string fallback token is represented as invalid for
        // the narrow untyped subset. `parse_nested_block` still enforces that
        // no additional tokens remain in the function.
        input.expect_ident().ok()?;
        None
    };
    Some(ContentComponent::AttrFallback { name, fallback })
}

/// CSS `<url>` value type (CSS Values and Units 4 §4.4
/// <https://www.w3.org/TR/css-values-4/#urls>) を一般 property value として
/// 受理する共通 helper。
///
/// Grammar: `<url> = <url()> | <src()>`、
/// `<url()> = url( <string> <url-modifier>* ) | <url-token>`。本 helper は
/// `<url()>` の 2 形式のみを受理する — unquoted `url(foo.png)` (tokenizer が
/// 生成する `<url-token>`) と、quoted `url("foo.png")` (tokenizer は quote を
/// 見て `url` を function token として切り出し、nested block 内の
/// `<string-token>` を読む)。`<url-modifier>*` (`crossorigin()` 等、§4.4.1) を
/// 伴う `url()` は nested block が `<string>` 単体で exhaust しないため
/// 全体が reject される — cssparser の block-exhaustion 制約
/// (`Parser::parse_nested_block` は closure が block を完全消費しないと Err)
/// によるもので、意図的な未対応。`<src()>` は別理由で未対応 —
/// `expect_url()` は function token 名を `url` に限定するため、`src(...)` は
/// そもそも function token としてすら認識されず reject される
/// (block-exhaustion 制約とは無関係)。
///
/// 一般 property value としての `<url>` はこの 2 形式のみで、bare
/// `<string>` (quote だけで `url()` wrapper を伴わない形) は含まない — spec
/// 本文が `"Some CSS contexts (such as @import) also allow a <url> to be
/// represented by a bare <string>, without the function wrapper"` と明記する
/// 通り、bare string 受理は `@import` 等の特定 context に限定された legacy
/// 挙動であり、一般 property の `<url>` value type には及ばない。
/// (`[ <string> | <url> ]` を独立した alternative として明示的に持つ property
/// は [`parse_target_url`] のように呼び出し側で `<string>` を別途扱う。)
///
/// CSS-wide keyword (`inherit` 等の bare ident) はこの grammar のどの形にも
/// 一致しないため cssparser の `expect_url` が Err を返し、本 helper も
/// `None` を返す — 呼び出し元の `parse_value` 規約 (`None` で declaration
/// 全体を drop) と自然に合致する。
// `pub(crate)` ではなく plain `fn`: 呼び出し元 (`parse_background_image`) は
// property.rs 内に実装されており、外部 module からの呼び出し元は無い —
// 外部呼び出し元が実際に landing した時点で
// `parse_non_negative_length`/`parse_length_allow_negative` (このファイル内、
// 外部呼び出し元を doc に明記した precedent) と同様の `pub(crate)` + doc
// justification へ拡張する。
pub(crate) fn parse_url_value(input: &mut Parser<'_, '_>) -> Option<String> {
    input.expect_url().ok().map(|s| s.as_ref().to_string())
}

/// target-* の第 1 引数 `[ <string> | <url> ]` を raw String として抽出。
/// `url("...")` / `url(...)` / bare `"..."` を統一的に受ける
/// (cssparser の `expect_url_or_string` を使用)。`<url>` 側の 2 形式は
/// [`parse_url_value`] と共通だが、bare `<string>` alternative も grammar に
/// 含む点が一般 `<url>` value type と異なるため、専用 helper として分離する。
fn parse_target_url(input: &mut Parser<'_, '_>) -> Option<String> {
    input
        .expect_url_or_string()
        .ok()
        .map(|s| s.as_ref().to_string())
}

/// `target-counter([<string>|<url>], <custom-ident>, <counter-style>?)`。
/// CSS Content 3 §2.6.1 <https://www.w3.org/TR/css-content-3/#target-counter>。
fn parse_target_counter_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounter { url, name, style })
}

/// `target-counters([<string>|<url>], <custom-ident>, <string>, <counter-style>?)`。
/// CSS Content 3 §2.6.2 <https://www.w3.org/TR/css-content-3/#target-counters>。
fn parse_target_counters_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    input.expect_comma().ok()?;
    let name = parse_custom_ident(input)?;
    input.expect_comma().ok()?;
    let separator = input.expect_string().ok()?.as_ref().to_string();
    let style = parse_optional_counter_style(input)?;
    Some(ContentComponent::TargetCounters {
        url,
        name,
        separator,
        style,
    })
}

/// `target-text([<string>|<url>], [ content | before | after | first-letter ]?)`。
/// CSS Content 3 §2.6.3 <https://www.w3.org/TR/css-content-3/#target-text>。
fn parse_target_text_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let url = parse_target_url(input)?;
    let part = if input.try_parse(|i| i.expect_comma()).is_ok() {
        parse_content_part(input)?
    } else {
        ContentPart::default()
    };
    Some(ContentComponent::TargetText { url, part })
}

/// `position: static | sticky | running(<custom-ident>)` を parse する
/// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax> および
/// CSS Positioned Layout Module Level 3 §3
/// <https://www.w3.org/TR/css-position-3/#sticky-pos>)。
///
/// 現状 scope:
/// - `static` — [`PositionValue::Static`]、`inherit_from` の初期状態と一致するため
///   apply_value が no-op でも問題ない。cascade winner selection では
///   先行 `running(...)` を上書き suppress する identity 用途
///   (standalone-static test だけでは実効性が問えない点に注意)。
/// - `sticky` — [`PositionValue::Sticky`]、CSS Positioned Layout §3。
///   現状は parse のみ受理し `apply_value` は `Static` 同様に no-op (将来の
///   layout 連携まで保持する)。
/// - `running(<custom-ident>)` — [`PositionValue::Running`]、apply_value が
///   1-item `RunningTemplate` を computed.running_templates に seed する。
/// - 他 keyword (`relative` / `absolute` / `fixed`) は未実装、silent drop = `None`。
///
/// `<custom-ident>` の除外は string-set と同じ規約:
/// [`is_reserved_custom_ident`] (CSS-wide keyword + `default`) に加えて
/// `none` を弾く。`none` は position property の他 spec-defined keyword
/// では無いが、custom-ident としては予約 alternative の慣行を残しつつ、
/// runtime resolve で `element(none)` 参照を誤って matching させないためのガード
/// (string-set の `none` reject と同じ扱い)。
fn parse_position(input: &mut Parser<'_, '_>) -> Option<PositionValue> {
    // `static` は現状 scope で受理する keyword の一つ。
    if input
        .try_parse(|i| i.expect_ident_matching("static"))
        .is_ok()
    {
        return Some(PositionValue::Static);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("relative"))
        .is_ok()
    {
        return Some(PositionValue::Relative);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("absolute"))
        .is_ok()
    {
        return Some(PositionValue::Absolute);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("fixed"))
        .is_ok()
    {
        return Some(PositionValue::Fixed);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("sticky"))
        .is_ok()
    {
        return Some(PositionValue::Sticky);
    }
    // `running(<custom-ident>)`。function name は ASCII case-insensitive、
    // 中身の custom-ident は case-preserving で SmolStr に格納。
    let running = input.try_parse(|i| -> Result<SmolStr, ParseError<'_, ()>> {
        let fn_name = i.expect_function()?.clone();
        if !fn_name.eq_ignore_ascii_case("running") {
            return Err(i.new_custom_error(()));
        }
        i.parse_nested_block(|inner| -> Result<SmolStr, ParseError<'_, ()>> {
            let ident = inner.expect_ident()?.clone();
            if is_reserved_custom_ident(&ident) || ident.eq_ignore_ascii_case("none") {
                return Err(inner.new_custom_error(()));
            }
            Ok(SmolStr::new(ident.as_ref()))
        })
    });
    running.ok().map(PositionValue::Running)
}

/// `top` / `right` / `bottom` / `left: auto | <length-percentage>` を parse する
/// (CSS Positioned Layout Module Level 3 §3).
fn parse_inset(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, false)?;
    Some(LengthOrAuto::Length(length))
}

/// `z-index: auto | <integer>` を parse する (CSS2 §9.9.1
/// <https://www.w3.org/TR/CSS2/visuren.html#z-index>, [`ZIndexValue`] doc
/// 参照)。
///
/// `auto` ident branch を先に try_parse する — [`parse_margin_side`] と同じ
/// order-of-alternative 理由 (同関数 doc 参照)、ここでは the two branches
/// (`auto` ident と integer token) の token kind が既に不連続なので必須では
/// ないが、既存 sibling と同じ並びに揃える。
///
/// integer 本体は [`parse_counter_property`] の `<integer>` 抽出と同じ
/// `expect_integer` 直接呼び出し — CSS Values 3 §4.2 "Integers: the
/// `<integer>` type" により符号付き (負値含む) を許容し、range 制限は無い。
fn parse_z_index(input: &mut Parser<'_, '_>) -> Option<ZIndexValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ZIndexValue::Auto);
    }
    input
        .try_parse(|i| i.expect_integer())
        .ok()
        .map(ZIndexValue::Integer)
}

/// `orphans` / `widows: <integer>` の value を parse する (CSS Fragmentation
/// Module Level 3 §3.3 "Breaks Between Lines: orphans, widows"
/// <https://www.w3.org/TR/css-break-3/#widows-orphans>).
///
/// `expect_integer` 直接呼び出しは [`parse_z_index`] / [`parse_counter_property`]
/// と同じ pattern。この 2 property は `<integer>` を **正の値のみ**に制限する
/// spec 独自の制約を持つ点が sibling と異なる: "Only positive integers are
/// allowed as values of orphans and widows. Negative values and zero are
/// invalid and must cause the declaration to be ignored." — 0 以下は
/// spec-invalid として `None` (declaration が丸ごと drop される、
/// 他の out-of-range `<integer>`/`<length>` value と同じ扱い)。
fn parse_positive_integer(input: &mut Parser<'_, '_>) -> Option<i32> {
    let value = input.try_parse(|i| i.expect_integer()).ok()?;
    if value > 0 { Some(value) } else { None }
}

/// `column-count: auto | <integer [1,∞]>`.
fn parse_column_count(input: &mut Parser<'_, '_>) -> Option<ColumnCountValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ColumnCountValue::Auto);
    }
    let count = parse_positive_integer(input)?;
    Some(ColumnCountValue::Count(u32::try_from(count).ok()?))
}

/// `column-width: auto | <length [0,∞]>`.
fn parse_column_width(input: &mut Parser<'_, '_>) -> Option<ColumnWidthValue> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(ColumnWidthValue::Auto);
    }
    Some(ColumnWidthValue::Length(parse_non_negative_length(input)?))
}

fn parse_columns_shorthand(input: &mut Parser<'_, '_>) -> Option<ColumnsShorthand> {
    // The grammar is an unordered pair (`||`).  Parsing in one fixed order
    // is not enough because `auto` is valid for both components: `auto 3`
    // and `auto 200px` need opposite interpretations of the first token.
    // Each complete candidate is tried transactionally and must consume the
    // whole value, so duplicate components and trailing garbage are rejected.
    if let Ok(value) = input.try_parse(|i| {
        let width = parse_column_width(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?;
        let has_second = !i.is_exhausted();
        let count = if has_second {
            parse_column_count(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?
        } else {
            ColumnCountValue::Auto
        };
        if !i.is_exhausted() {
            return Err(i.new_custom_error::<(), ()>(()));
        }
        Ok(ColumnsShorthand { count, width })
    }) {
        return Some(value);
    }

    input
        .try_parse(|i| {
            let count = parse_column_count(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?;
            let has_second = !i.is_exhausted();
            let width = if has_second {
                parse_column_width(i).ok_or_else(|| i.new_custom_error::<(), ()>(()))?
            } else {
                ColumnWidthValue::Auto
            };
            if !i.is_exhausted() {
                return Err(i.new_custom_error::<(), ()>(()));
            }
            Ok(ColumnsShorthand { count, width })
        })
        .ok()
}

/// `break-before: <ident>` / `break-after: <ident>` を parse する (CSS
/// Fragmentation Module Level 3 §3.1
/// <https://www.w3.org/TR/css-break-3/#break-between>)。
///
/// この crate の scope で受理する 4 keyword ([`BreakBetween`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page` / `page`。propdef の
/// 残り 8 keyword (`left` / `right` / `recto` / `verso` / `avoid-column` /
/// `column` / `avoid-region` / `region`) と、現行 spec grammar に無い
/// `always` / `all` は他の未知 ident と同じく silent drop (`None`)。ASCII
/// case-insensitive で ident を比較する ([`parse_word_break`] 等 sibling と
/// 同 flavor)。
fn parse_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "avoid-page" => Some(BreakBetween::AvoidPage),
        "page" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `break-inside: <ident>` を parse する (CSS Fragmentation Module Level 3
/// §3.2 <https://www.w3.org/TR/css-break-3/#break-within>)。
///
/// この crate の scope で受理する 3 keyword ([`BreakInside`] doc の Scope
/// carving 節参照): `auto` / `avoid` / `avoid-page`。propdef の残り 2
/// keyword (`avoid-column` / `avoid-region`) は他の未知 ident と同じく
/// silent drop (`None`)。ASCII case-insensitive で ident を比較する
/// ([`parse_break_between`] と同 flavor)。
fn parse_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        "avoid-page" => Some(BreakInside::AvoidPage),
        _ => None,
    }
}

/// `page-break-before: <ident>` / `page-break-after: <ident>` — CSS2.1
/// legacy shorthand for `break-before` / `break-after` — を parse し、
/// [`BreakBetween`] へ remap する (CSS Fragmentation Module Level 3 §3.4
/// <https://www.w3.org/TR/css-break-3/#page-break-properties>,
/// [`BreakBetween`] doc の「legacy shorthand」節の mapping table 参照)。
///
/// CSS2.1 自身の `page-break-before` / `page-break-after` propdef grammar
/// (verbatim, <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-before>)
/// は `auto | always | avoid | left | right`。本 parser はそのうち
/// `auto` / `avoid` / `always` の 3 keyword のみ受理する — `left` /
/// `right` は `break-before`/`break-after` 側で未実装 ([`BreakBetween`] doc
/// の Scope carving 節) の値へ remap されるため、この legacy shorthand
/// 経由でも同じく受理しない。`always` は spec の mapping table どおり
/// [`BreakBetween::Page`] へ remap する (identity ではない — `auto` /
/// `avoid` は identity)。
fn parse_legacy_page_break_between(input: &mut Parser<'_, '_>) -> Option<BreakBetween> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakBetween::Auto),
        "avoid" => Some(BreakBetween::Avoid),
        "always" => Some(BreakBetween::Page),
        _ => None,
    }
}

/// `page-break-inside: <ident>` — CSS2.1 legacy shorthand for
/// `break-inside` — を parse する (CSS Fragmentation Module Level 3 §3.4,
/// [`BreakInside`] doc の「legacy shorthand」節参照)。
///
/// CSS2.1 自身の `page-break-inside` propdef grammar (verbatim,
/// <https://www.w3.org/TR/CSS2/page.html#propdef-page-break-inside>) は
/// `avoid | auto` のみ (`always` / `left` / `right` は無い) — この 2
/// keyword を [`BreakInside`] へ identity mapping する。`break-inside`
/// 自身が持つ `avoid-page` は CSS2.1 の `page-break-inside` grammar には
/// 無いため受理しない。
fn parse_legacy_page_break_inside(input: &mut Parser<'_, '_>) -> Option<BreakInside> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(BreakInside::Auto),
        "avoid" => Some(BreakInside::Avoid),
        _ => None,
    }
}

fn parse_content_part(input: &mut Parser<'_, '_>) -> Option<ContentPart> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "content" => Some(ContentPart::Content),
        "before" => Some(ContentPart::Before),
        "after" => Some(ContentPart::After),
        "first-letter" => Some(ContentPart::FirstLetter),
        _ => None,
    }
}

/// `content([ text | before | after | first-letter ]?)` (`?` は raikiri の
/// 受理済み記法であり、GCPM 3 の grammar 自体には無い formal optional
/// marker ではない)。CSS GCPM 3 §1.1.1.1 "The content() function"
/// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content> の keyword 集合
/// (`text | before | after | first-letter` の 4 種) をそのまま実装する。
///
/// **grammar 選択の根拠**: `content()` は GCPM 3 §1.1.1.1 と CSS Content 3
/// §2.7.3 <https://www.w3.org/TR/css-content-3/#funcdef-content> の 2 つの
/// spec に別々に定義されており、2 つの軸で食い違う。(1) keyword 集合 — GCPM 3
/// は 4 keyword のみ、CSS Content 3 はそこに `marker` を加えた 5 keyword。
/// (2) 引数の省略可否 — GCPM 3 の production 自体には `?` が無く引数は形式上
/// 必須だが、CSS Content 3 は `?` 付きで、省略時は `text` を暗黙採用すると
/// 明記する。この実装は (1) の keyword 集合では GCPM 3 §1.1.1.1 に従い、
/// `marker` を意図的に reject する。(2) の引数省略可否については逆に
/// CSS Content 3 §2.7.3 の `?` 付き grammar と同じ挙動 (省略時 `text`
/// フォールバック) を採用しており、GCPM 3 の厳密な grammar (引数必須) には
/// 従っていない — 「GCPM 3 に従う」と言えるのは keyword 集合の軸のみである。
/// これは spec 間の grammar 相反を軸ごとに解決した結果の選択であり、
/// 実装漏れではない。
///
/// **既知の feature gap**: `content` property 側の `<content-list>` は
/// CSS Content 3 §2 governance (broad grammar、[`ContentListMode::CssContent3`]
/// 参照) だが、この `content()` 内部の keyword 集合だけは両 property 呼び出し
/// 元で GCPM 3 §1.1.1.1 の 4-keyword 版のまま unconditional に適用される
/// (下記 mode dispatch の節参照)。引数省略時の `text` フォールバック挙動は
/// 既に CSS Content 3 §2.7.3 の記述と一致しているため、CSS Content 3 §2.7.3
/// の広い grammar を優先実装する必要が生じた場合、残る差分は `marker`
/// keyword の受理のみ。
///
/// bare `content()` (spec 例 `h2 { string-set: heading content() }`、
/// string-set/GCPM3 側の文脈) では [`ContentTextKeyword::Text`] を
/// フォールバック値として使う (根拠は GCPM 3 側の spec "default" 宣言では
/// ない — 詳細は [`ContentTextKeyword`] の doc comment 参照)。target-text()
/// の第 2 引数と
/// 違い、keyword は paren 直下に置かれる (comma を先行させない)。
///
/// GCPM 3 §1.1.1 の narrow `<content-list>` (string-set 側) と CSS Content 3
/// §2 の broad `<content-list>` (content property 側) の **両方** に対し
/// unconditional に受理される ([`ContentListMode`] mode gate なし、
/// mode dispatch 導入後もこの arm は両 mode で unconditional のまま、
/// [`parse_content_function`] の match arm 参照) — 上記の通り、この
/// unconditional な適用自体が「受理 keyword 集合は両 property とも
/// GCPM 3 §1.1.1.1 の 4 種」という選択の実装箇所である。
fn parse_content_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let keyword = if input.is_exhausted() {
        ContentTextKeyword::default()
    } else {
        parse_content_text_keyword(input)?
    };
    Some(ContentComponent::Content { keyword })
}

fn parse_content_text_keyword(input: &mut Parser<'_, '_>) -> Option<ContentTextKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "text" => Some(ContentTextKeyword::Text),
        "before" => Some(ContentTextKeyword::Before),
        "after" => Some(ContentTextKeyword::After),
        "first-letter" => Some(ContentTextKeyword::FirstLetter),
        _ => None,
    }
}

/// `leader(<leader-type>)`。CSS Content 3 §2.5.1 "The leader() function"
/// <https://www.w3.org/TR/css-content-3/#leader-function>。spec production
/// `leader( <leader-type> )` に `?` が無いため引数は必須
/// (`parse_leader_type` 失敗 = declaration drop、`leader()` 単体は spec-invalid)。
fn parse_leader_fn(input: &mut Parser<'_, '_>) -> Option<ContentComponent> {
    let leader_type = parse_leader_type(input)?;
    Some(ContentComponent::Leader(leader_type))
}

/// `<leader-type> = dotted | solid | space | <string>`。[`LeaderType`] の doc
/// も参照 (keyword を正規化せず個別 variant で保持する rationale)。
fn parse_leader_type(input: &mut Parser<'_, '_>) -> Option<LeaderType> {
    if let Ok(s) = input.try_parse(|i| i.expect_string_cloned()) {
        return Some(LeaderType::String(SmolStr::new(s.as_ref())));
    }
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "dotted" => Some(LeaderType::Dotted),
        "solid" => Some(LeaderType::Solid),
        "space" => Some(LeaderType::Space),
        _ => None,
    }
}

/// `text-shadow: <color>? && <length>{2,3}` 成分の `<color>` slot。
///
/// [`parse_border_color`] / [`parse_text_decoration_color`] と同型 —
/// `currentcolor` keyword (CSS Color 3 §4.4) を先取りしてから
/// [`parse_color`] (hex / named / `rgb(a)` / `transparent`) に委譲する。
/// 独立した helper にしてあるのは、他 2 者と異なる payload 型
/// ([`TextShadowColor`]) を持つため。
fn parse_text_shadow_color(input: &mut Parser<'_, '_>) -> Option<TextShadowColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(TextShadowColor::CurrentColor);
    }
    parse_color(input).map(TextShadowColor::Resolved)
}

/// `<length>{2,3}` の contiguous run (offset-x offset-y blur-radius?) を 1
/// unit として parse する — [`TextShadowItem`] doc の「Non-negative
/// blur-radius」節参照。
///
/// offset-x/offset-y は [`parse_shadow_length_reject_nan`] 経由 — 同関数の
/// doc が説明する `!is_nan()` guard を通す (sign 制限が無いため
/// blur-radius 側の `>= 0.0` incidental filter が効かない)。
///
/// blur-radius は [`parse_non_negative_length`] で non-negative を
/// enforce、省略時は `Length::Px(0.0)` (同 doc の「各成分の初期値埋め」節)。
/// caller ([`parse_text_shadow_item`]) が本関数全体を `try_parse` で包む
/// ことで、offset-x の parse 失敗 (= 最初の token が `<color>` 等) 時に
/// x/y いずれの消費も正しく rewind される
/// ([`parse_length_value`] は token を unconditional に消費するため、
/// [`parse_margin_side`] doc の「Order of alternative」節と同じ理由で
/// checkpoint 経由の rewind が要る)。
pub(crate) fn parse_text_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length, Length), ParseError<'i, ()>> {
    let x = parse_shadow_length_reject_nan_res(input)?;
    let y = parse_shadow_length_reject_nan_res(input)?;
    let blur = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_non_negative_length(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok((x, y, blur))
}

/// `text-shadow` の 1 shadow entry — `<color>? && <length>{2,3}`。
///
/// # `&&` (both-required, any-order) grammar semantics
///
/// spec CSS Values 4 §2.2 "Component Value Combinators"
/// <https://www.w3.org/TR/css-values-4/#component-combinators> verbatim: "A
/// double ampersand (&&) separates two or more components, all of which must
/// occur, in any order." — 本 grammar では:
/// - length run (`<length>{2,3}`) は必須、1 回のみ
/// - `<color>` はその自身の `?` multiplier により 0 or 1 回
/// - 両者の順序は自由 (`1px 1px red` / `red 1px 1px` 全て valid)、ただし
///   length run 自体は contiguous (`1px red 1px` のように間へ `<color>` を
///   挟むことはできない — [`parse_text_shadow_lengths`] が 1 unit として
///   parse する)
///
/// # Loop 実装
///
/// [`parse_border_shorthand`] の `||` loop と同じ shape — unfilled slot
/// (length run / color) を loop で peel、`try_parse` で order-independent に
/// 試す。length run を先に試すのは任意の順序選択 (どちらを先に試しても
/// 結果は変わらない、`try_parse` が失敗時に必ず rewind するため)。
fn parse_text_shadow_item(input: &mut Parser<'_, '_>) -> Option<TextShadowItem> {
    let mut lengths: Option<(Length, Length, Length)> = None;
    let mut color: Option<TextShadowColor> = None;

    loop {
        if lengths.is_some() && color.is_some() {
            break;
        }
        if lengths.is_none()
            && let Ok(triple) = input.try_parse(parse_text_shadow_lengths)
        {
            lengths = Some(triple);
            continue;
        }
        if color.is_none()
            && let Ok(c) = input.try_parse(|i| -> Result<TextShadowColor, ParseError<'_, ()>> {
                parse_text_shadow_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(c);
            continue;
        }
        break;
    }

    // length run は必須 (spec grammar 上 `<length>{2,3}` に `?` が無い) —
    // 0 slot でも `<color>` だけが埋まる可能性は無いが、`parse_border_shorthand`
    // の「at least 1 component 必須」とは違い、本 grammar では length run
    // 単独でも valid (`<color>` は完全に optional)。
    let (offset_x, offset_y, blur_radius) = lengths?;
    Some(TextShadowItem {
        offset_x,
        offset_y,
        blur_radius,
        color: color.unwrap_or(TextShadowColor::CurrentColor),
    })
}

/// `text-shadow: none | <shadow>#` を parse する。
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property>。
///
/// `none` = 空 list (top-level alternative) — [`parse_content`] /
/// [`parse_counter_property`] と同じ shape。comma-separated list は
/// `cssparser::Parser::parse_comma_separated` に委譲 (各 item の未消費
/// leftover token は同メソッドの `parse_until_before` → `parse_entirely`
/// が自動検知して declaration ごと drop する — [`TextShadowItem`] doc の
/// 「Non-negative blur-radius」節が挙げる `text-shadow: 1px 1px -1px`
/// (負の blur-radius は unconsumed のまま残る) のような入力はこの経路で
/// reject される)。
fn parse_text_shadow(input: &mut Parser<'_, '_>) -> Option<Vec<TextShadowItem>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    input
        .parse_comma_separated(|i| -> Result<TextShadowItem, ParseError<'_, ()>> {
            parse_text_shadow_item(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// `border-radius` の 1--4 個の circular `<length>` を四隅へ展開する。
///
/// CSS Backgrounds and Borders 3 §5 の shorthand expansion に従い、値は
/// top-left, top-right, bottom-right, bottom-left の順で解釈する。
/// percentage は受理するが、slash 以降の楕円形指定と負値は受理しない。
fn parse_border_radius(input: &mut Parser<'_, '_>) -> Option<BorderRadius> {
    let first = parse_non_negative_length_percentage(input)?;
    let second = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let third = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();
    let fourth = input
        .try_parse(parse_non_negative_length_percentage_res)
        .ok();

    Some(match (second, third, fourth) {
        (None, _, _) => BorderRadius {
            top_left: first,
            top_right: first,
            bottom_right: first,
            bottom_left: first,
        },
        (Some(opposite), None, _) => BorderRadius {
            top_left: first,
            top_right: opposite,
            bottom_right: first,
            bottom_left: opposite,
        },
        (Some(horizontal), Some(bottom), None) => BorderRadius {
            top_left: first,
            top_right: horizontal,
            bottom_right: bottom,
            bottom_left: horizontal,
        },
        (Some(top_right), Some(bottom_right), Some(bottom_left)) => BorderRadius {
            top_left: first,
            top_right,
            bottom_right,
            bottom_left,
        },
    })
}

fn parse_non_negative_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    parse_non_negative_length_percentage_res(input).ok()
}

fn parse_length_allow_negative_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_allow_negative(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<length>` for shadow offset-x/offset-y (`text-shadow`/`box-shadow`) and
/// box-shadow's spread-radius — [`parse_length_allow_negative`]'s
/// unrestricted-sign shape (negative offsets/spread are valid per the two
/// properties' shared `<shadow>` grammar, CSS Backgrounds 3 §6.1 / CSS Text
/// Decoration Module Level 3 §4), with an explicit `!is_nan()` guard layered
/// on top.
///
/// # Why this guard is (mostly) already a no-op
///
/// A huge-exponent literal like `0e999px` has an exact mathematical value
/// of `0` (CSS Syntax 3 §4.3.13's own `<number-token>` conversion is
/// `sign * mantissa * 10^exponent`, and `0 * anything` is `0`), but
/// cssparser 0.37.0's tokenizer computes that same formula as a separate
/// floating-point step (`mantissa * 10^exponent`) that collapses to `NaN`
/// for this input (module doc's "Numeric-token NaN stabilization"
/// section). `parse_length_value` — which `parse_length_allow_negative`
/// (and so this function) goes through — already routes its token
/// acquisition through `next_numeric_stable`, which corrects exactly this
/// class of `NaN` before this function ever sees the `Length`. So in
/// ordinary use `length_payload(length)` here is never `NaN` for a
/// zero-mantissa literal; this `!is_nan()` check is kept as
/// defense-in-depth, same precedent as [`parse_opacity_value`]'s guard.
///
/// # Why the guard must be explicit here
///
/// [`parse_non_negative_length`]'s `>= 0.0` filter incidentally also
/// rejects NaN, but offset-x/offset-y/spread-radius carry no sign
/// restriction — unlike blur-radius (`[0,∞]`, see
/// [`parse_text_shadow_lengths`]/[`parse_box_shadow_lengths`]) — so there
/// is no such incidental filter here; the guard must be explicit, same
/// shape [`parse_transform_length_percentage`] uses for `transform`'s own
/// unconsumed payloads.
///
/// # `+Inf`/`-Inf` are not rejected
///
/// No paint-side consumer applies shadow offsets/spread to anything yet,
/// so nothing downstream in this crate will ever normalize a NaN that
/// slips past this parser. `+Inf`/`-Inf`, by contrast, are ordinary
/// `<length>` magnitude overflow — legitimate, if extreme, values per CSS
/// Values 4 §5's "closest value supported by the implementation" clause —
/// and are preserved unfiltered.
fn parse_shadow_length_reject_nan(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_allow_negative(input)?;
    (!length_payload(length).is_nan()).then_some(length)
}

fn parse_shadow_length_reject_nan_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_shadow_length_reject_nan(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<length-percentage>` — sign 制限なし ([`parse_length_value`] with
/// `allow_percentage=true`、`Result` wrapper for `try_parse` callers)。
/// [`CssPosition`] の offset (負値も spec-valid、[`parse_position_horizontal_edge`]
/// 等の doc 参照) が使う。
fn parse_length_percentage_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))
}

/// [`CssPositionOffset::End`] の `Percent` payload を、等価な
/// [`CssPositionOffset::Start`] へ畳む ([`CssPositionOffset`] doc の
/// 「なぜ 2 variant か」節)。`right 30%` (`End(Percent(30.0))`) は
/// `Start(Percent(70.0))` と完全に等価な値なので、常に `Start` 側へ
/// 正規化して代表形を 1 つに保つ — `End` が生き残るのは percentage で
/// 表現できない offset (`right 10px` 等) に限られる。
fn normalize_css_position_offset(offset: CssPositionOffset) -> CssPositionOffset {
    match offset {
        CssPositionOffset::End(Length::Percent(p)) => {
            CssPositionOffset::Start(Length::Percent(100.0 - p))
        }
        other => other,
    }
}

pub(crate) fn normalize_css_position(position: CssPosition) -> CssPosition {
    CssPosition {
        horizontal: normalize_css_position_offset(position.horizontal),
        vertical: normalize_css_position_offset(position.vertical),
    }
}

fn css_position_center() -> CssPositionOffset {
    CssPositionOffset::Start(Length::Percent(50.0))
}

/// `<position>` grammar ([`CssPosition`] doc) の edge + optional offset
/// を読む共通ヘルパー。`start_kw` (`left`/`top`) は [`CssPositionOffset::Start`]、
/// `end_kw` (`right`/`bottom`) は [`CssPositionOffset::End`] にマップする。
/// 戻り値の 2nd 要素の意味は [`parse_position_horizontal_edge`] doc 参照。
fn parse_position_edge(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::Start(offset), true),
            Err(_) => (CssPositionOffset::Start(Length::Percent(0.0)), false),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(match input.try_parse(parse_length_percentage_res) {
            Ok(offset) => (CssPositionOffset::End(offset), true),
            Err(_) => (CssPositionOffset::End(Length::Percent(0.0)), false),
        });
    }
    None
}

/// `<position>` grammar ([`CssPosition`] doc) の `[ left | right ]
/// <length-percentage>?` alternative — horizontal 軸の edge keyword を
/// optional offset とともに読む。offset 省略時は edge そのもの (offset
/// `0`) を返す。`left`/`right` どちらにもマッチしなければ `None`
/// (token は消費しない)。
///
/// 戻り値の 2nd 要素は「`<length-percentage>` token が実際に authored
/// されていたか (offset 省略時の暗黙 `0` ではない)」— [`parse_position_branch3`]
/// (`<bg-position>` 用、`background-position` が使う) はこれを無視するが、
/// [`parse_position_branch3_strict`] (plain `<position>` 用、`object-position`
/// が使う) はこれを使って 3-value 形式 (offset がどちらか片方の軸にだけ
/// authored されている状態、`<position>` doc の Grammar 節参照) を検出・
/// reject する。
fn parse_position_horizontal_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "left", "right")
}

/// [`parse_position_horizontal_edge`] の vertical 軸版 (`top`/`bottom`) —
/// 戻り値の 2nd 要素の意味は同関数の doc 参照。
fn parse_position_vertical_edge(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    parse_position_edge(input, "top", "bottom")
}

/// `center | [ left | right ] <length-percentage>?` — horizontal 軸の
/// branch-3 group ([`parse_position_branch3`] doc)。`center` は
/// offset を持たないため 2nd 要素は常に `false`
/// ([`parse_position_horizontal_edge`] doc参照)。
fn parse_position_horizontal_group(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_horizontal_edge(input)
}

/// [`parse_position_horizontal_group`] の vertical 軸版。
fn parse_position_vertical_group(input: &mut Parser<'_, '_>) -> Option<(CssPositionOffset, bool)> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some((css_position_center(), false));
    }
    parse_position_vertical_edge(input)
}

/// [`parse_position_branch3`] と [`parse_position_branch3_strict`] の共通コア —
/// `<position>` 3rd alternative の構造を一度だけ記述し、offset 有無
/// (`bool` 2 つ) を呼び出し元へ返す。`center` は常に offset なし (`false`)
/// ([`parse_position_horizontal_group`] doc 参照)。
fn parse_position_branch3_core(input: &mut Parser<'_, '_>) -> Option<(CssPosition, bool, bool)> {
    if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
        let (vertical, v_offset) = parse_position_vertical_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
        let (horizontal, h_offset) = parse_position_horizontal_group(input)?;
        return Some((
            CssPosition {
                horizontal,
                vertical,
            },
            h_offset,
            v_offset,
        ));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        if let Some((vertical, v_offset)) = parse_position_vertical_edge(input) {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical,
                },
                false,
                v_offset,
            ));
        }
        if let Some((horizontal, h_offset)) = parse_position_horizontal_edge(input) {
            return Some((
                CssPosition {
                    horizontal,
                    vertical: css_position_center(),
                },
                h_offset,
                false,
            ));
        }
        if input
            .try_parse(|i| i.expect_ident_matching("center"))
            .is_ok()
        {
            return Some((
                CssPosition {
                    horizontal: css_position_center(),
                    vertical: css_position_center(),
                },
                false,
                false,
            ));
        }
        return None;
    }
    None
}

/// `<position>` grammar ([`CssPosition`] doc) の 3rd alternative —
/// `[ center | [ left | right ] <length-percentage>? ] && [ center | [ top |
/// bottom ] <length-percentage>? ]`。`&&` は両 group が (任意順で) 必須
/// なことを意味する — 一方の group しか読めなければ (呼び出し元が
/// `input.try_parse` で包む前提の) `None` を返し、消費した token は
/// 呼び出し元の rewind に委ねる。
///
/// 1 個目の token は horizontal-exclusive (`left`/`right`) → vertical-exclusive
/// (`top`/`bottom`) → ambiguous `center` の順で試す。`center` は両 group に
/// 属し得るため、どちらの軸に属するかは 2 個目の token (もう片方の
/// group) を見て初めて決まる。
///
/// これは `<bg-position>` (CSS Backgrounds 3 §2.6) の 3rd alternative
/// そのもの — 各 group の `<length-percentage>?` を独立に optional として
/// 扱う (offset がどちらか片方の軸にだけ authored される 3-value 形式を
/// 受理する)。plain `<position>` (CSS Values 4 §8.3) 向けにはこの中間形を
/// reject する [`parse_position_branch3_strict`] を使うこと —
/// `background-position` (`<bg-position>` を要求) はこちら、
/// `object-position` (`<position>` を要求) はあちら、という使い分けが
/// [`CssPosition`] の Grammar 節の canonical な説明。
fn parse_position_branch3(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, _, _) = parse_position_branch3_core(input)?;
    Some(position)
}

/// [`parse_position_branch3`] の plain-`<position>` 版 —
/// 唯一の違いは、horizontal/vertical 両 group の `<length-percentage>`
/// offset **有無が一致しない場合 (3-value 形式) を reject** する点。
///
/// # Why
///
/// CSS Values 4 §8.3 の `<position>` 自体には「offset がどちらか片方の
/// 軸にだけ authored される」中間形が存在しない — その 4th alternative
/// (`<position-four>`、MDN "`<position>` CSS type" の formal syntax 参照)
/// は `[[left|right] <length-percentage>] && [[top|bottom]
/// <length-percentage>]` であり、offset は `?` ではなく両軸とも必須。
/// offset を一切伴わない bare keyword pair は 2nd alternative
/// (`<position-two>` の `&&` 形) が別途カバーする。つまり有効な組み合わせは
/// 「両軸とも offset あり (4-value)」か「両軸とも offset なし」のみで、
/// 「片方だけ offset」は常に invalid。
///
/// `<bg-position>` (CSS Backgrounds 3 §2.6、`background-position` 用) は
/// この制約を持たない — 3-value 形式 ("For 3-value productions (which are
/// not valid in `<position>`)" と同 spec が明記) を明示的に許すのが
/// `<bg-position>` の `<position>` に対する拡張そのもの。`object-position`
/// (CSS Images 3 §5.2、Value: `<position>`) はこの拡張を持たないため、
/// [`parse_position_branch3`] をそのまま再利用すると `right 10px center`
/// のような 3-value 入力を誤って受理してしまう — token を過不足なく
/// 消費してしまう (leftover が残らない) ため、呼び出し元の
/// `expect_exhausted` による leftover 検出でも捕捉できない。本関数は
/// それを防ぐための、offset 有無の対称性チェックを追加した sibling。
pub(crate) fn parse_position_branch3_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let (position, h_offset, v_offset) = parse_position_branch3_core(input)?;
    if h_offset != v_offset {
        return None;
    }
    Some(position)
}

/// `[ <start_kw> | center | <end_kw> | <length-percentage> ]` — `<position>`
/// grammar ([`CssPosition`] doc) の 2nd alternative の axis 共通ヘルパー。
/// `start_kw` は `Start(0%)` (`left`/`top`)、`end_kw` は `Start(100%)`
/// (`right`/`bottom`) にマップする。bare `<length-percentage>` はこの
/// alternative でのみ受理される (branch3 の group はどちらも bare LP を
/// 単独では受理しない) — edge keyword が無いぶん offset ではなく「値そのもの」
/// として `Start` に格納する。
fn parse_position_branch2_axis(
    input: &mut Parser<'_, '_>,
    start_kw: &str,
    end_kw: &str,
) -> Option<CssPositionOffset> {
    if input
        .try_parse(|i| i.expect_ident_matching(start_kw))
        .is_ok()
    {
        return Some(CssPositionOffset::Start(Length::Percent(0.0)));
    }
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(css_position_center());
    }
    if input.try_parse(|i| i.expect_ident_matching(end_kw)).is_ok() {
        return Some(CssPositionOffset::Start(Length::Percent(100.0)));
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPositionOffset::Start(lp))
}

/// `[ left | center | right | <length-percentage> ]` — `<position>` grammar
/// ([`CssPosition`] doc) の 2nd alternative の horizontal 側。bare
/// `<length-percentage>` はこの alternative でのみ受理される (branch3 の
/// group はどちらも bare LP を単独では受理しない) — edge keyword が無い
/// ぶん offset ではなく「値そのもの」として `Start` に格納する。
fn parse_position_branch2_horizontal(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "left", "right")
}

/// [`parse_position_branch2_horizontal`] の vertical 側
/// (`[ top | center | bottom | <length-percentage> ]`).
fn parse_position_branch2_vertical(input: &mut Parser<'_, '_>) -> Option<CssPositionOffset> {
    parse_position_branch2_axis(input, "top", "bottom")
}

/// `<position>` grammar ([`CssPosition`] doc) の 2nd alternative — 厳密に
/// horizontal → vertical の順で 2 token を読む (keyword 並び替え不可、
/// branch3 と違い `&&` ではなく単純な連接)。
fn parse_position_branch2(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let horizontal = parse_position_branch2_horizontal(input)?;
    let vertical = parse_position_branch2_vertical(input)?;
    Some(CssPosition {
        horizontal,
        vertical,
    })
}

/// `<position>` grammar ([`CssPosition`] doc) の 1st alternative — 単一
/// keyword または単一 `<length-percentage>`。指定されなかった軸は
/// `center` (50%) になる (spec 明示なし、`background-position` (CSS
/// Backgrounds 3 §2.6) の "if only one value is specified, the second
/// value is assumed to be center" 相当を汎用 `<position>` 型として保持)。
fn parse_position_branch1(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    if input
        .try_parse(|i| i.expect_ident_matching("center"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("left")).is_ok() {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: css_position_center(),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("right"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: css_position_center(),
        });
    }
    if input.try_parse(|i| i.expect_ident_matching("top")).is_ok() {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("bottom"))
        .is_ok()
    {
        return Some(CssPosition {
            horizontal: css_position_center(),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        });
    }
    let lp = input.try_parse(parse_length_percentage_res).ok()?;
    Some(CssPosition {
        horizontal: CssPositionOffset::Start(lp),
        vertical: css_position_center(),
    })
}

/// `<position>` value type を parse する ([`CssPosition`] doc の grammar
/// 参照)。
///
/// # Alternative の試行順序 — 3rd → 2nd → 1st (grammar 記載順とは逆)
///
/// 3 alternative は互いに重なりうるため、試す順序が結果を左右する。
/// **3rd (edge 並び替え可) を最初に試す**理由 — 3rd の失敗が「安全に
/// rewind する」ことを保証するメカニズムの説明:
///
/// `left 10px` を例にとる。3rd alternative は horizontal group に `left`
/// を、続けて optional offset `10px` を貪欲に割り当てる — その結果
/// vertical group に残す token が無くなり (`&&` は両 group 必須)、3rd
/// alternative 全体が失敗して丸ごと rewind する (この入力に限れば 2nd を
/// 先に試しても `[left|center|right|<LP>]` が `left` を、
/// `[top|center|bottom|<LP>]` が残りの `10px` を bare
/// `<length-percentage>` として消費し、同じ horizontal = `left` (0%)、
/// vertical = `10px` に到達するため、この特定の入力だけでは順序は
/// 結果を左右しない — 3rd が「余計に消費してから失敗する」ことはあっても
/// 「誤った値で成功する」ことは無い、という下記の safety-invariant の
/// 具体例として引いている)。順序が真に結果を左右するのは `top left` の
/// ような keyword 並び替え入力 (2nd は horizontal→vertical の固定順しか
/// 受理しないため `top` を horizontal 側で reject し、3rd の `&&`
/// (任意順) でしか解釈できない) や、`bottom 10px right 20px` のような
/// 3-4 value edge-offset 入力 (2nd は高々 2 token しか消費しないため
/// leftover が残り `expect_exhausted` で丸ごと drop される) である。
///
/// 同じ理由で 2nd は 1st より先: 1st は token を 1 個しか消費しないため、
/// 2 token 以上の入力 (`0px 20px` 等) に対して 2nd を先に試さないと
/// 2 個目の value が leftover として残り、呼び出し元の `expect_exhausted`
/// (`rule.rs`) が declaration ごと drop してしまう。
///
/// この順序が「3rd が prefix だけ食って leftover を残す」場合でも安全な
/// 理由: 2nd は高々 2 token、1st は高々 1 token しか消費できないため、
/// 同じ開始位置から 2nd/1st が 3rd より **多くの** token を消費して
/// leftover をゼロにできることは無い — 3rd が成功した時点でそれが
/// 常に「最も leftover が少ない (またはゼロの)」alternative になる。
///
/// 各 alternative は `input.try_parse` で包まれた `Option`-returning
/// helper (`parse_position_branch3` 等) として実装し、途中まで token を
/// 消費して失敗しても呼び出し元の `try_parse` が丸ごと rewind する。
fn parse_position_branch3_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch2_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch2(input).ok_or_else(|| input.new_custom_error(()))
}

fn parse_position_branch1_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch1(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<bg-position>` (CSS Backgrounds 3 §2.6) value type を parse する — plain `<position>` (CSS Values 4 §8.3) の superset で、3-value edge-offset 形式を許す。`background-position` / `background` shorthand の position 部分 / gradient の `at <position>` 等がこの grammar を使う。素の `<position>` (3-value 形式を reject) は sibling の [`parse_position_strict`] を使うこと。
///
/// 3 alternative ([`parse_position_branch3`] / [`parse_position_branch2`] / [`parse_position_branch1`])
/// を 3rd → 2nd → 1st の順で試す (詳細は [`parse_position_branch3_res`] の alternative 順序 doc 参照)。
pub fn parse_bg_position(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

fn parse_position_branch3_strict_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<CssPosition, ParseError<'i, ()>> {
    parse_position_branch3_strict(input).ok_or_else(|| input.new_custom_error(()))
}

/// plain `<position>` (CSS Values 4 §8.3) value type を parse する —
/// [`parse_bg_position`] (`<bg-position>`、`background-position` 用) の
/// sibling。`object-position` (CSS Images 3 §5.2、Value: `<position>`) が
/// 使う。
///
/// 差分は 3rd alternative だけ — [`parse_position_branch3`] の代わりに
/// [`parse_position_branch3_strict`] を試す (3-value edge-offset 形式を
/// reject する、同関数 doc の "Why" 節参照)。2nd/1st alternative
/// ([`parse_position_branch2`]/[`parse_position_branch1`]) はどちらの
/// grammar でも同一なので共有する。
pub fn parse_position_strict(input: &mut Parser<'_, '_>) -> Option<CssPosition> {
    let position = input
        .try_parse(parse_position_branch3_strict_res)
        .or_else(|_| input.try_parse(parse_position_branch2_res))
        .or_else(|_| input.try_parse(parse_position_branch1_res))
        .ok()?;
    Some(normalize_css_position(position))
}

/// `background-image: <bg-image>` を parse する ([`BackgroundImage`] doc の
/// grammar 参照: `<image> | none`、`<image> = <url> | <gradient>`)。
///
/// `none` keyword を先に試し、次に `<gradient>` function を試す
/// (`<url>` 側 [`parse_url_value`] はどの function token にも一致しないため
/// 順序自体は結果を左右しないが、他の keyword-vs-function alternative を持つ
/// sibling parser — [`parse_position`] 等 — と並びを揃える)。
fn parse_background_image(input: &mut Parser<'_, '_>) -> Option<BackgroundImage> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(BackgroundImage::None);
    }
    if let Ok(gradient) = input.try_parse(parse_gradient) {
        return Some(BackgroundImage::Gradient(gradient));
    }
    parse_url_value(input).map(BackgroundImage::Url)
}

/// `mask-image: <mask-reference>` (single layer — [`MaskImage`] doc の
/// scope carving 節参照) を parse する。
///
/// grammar shape (`none | <image> | <mask-source>`、`<image> = <url> |
/// <gradient>`、`<mask-source> = <url>`) は [`parse_background_image`]'s
/// `<bg-image> = <url> | <gradient>` と concrete syntax レベルで一致する
/// ([`MaskImage`] doc の「`<mask-source>` と `<image>` の `url`
/// alternative は同じ具象構文」節) — [`BackgroundImage`] への alias により
/// body も共有する。
pub(crate) fn parse_mask_image(input: &mut Parser<'_, '_>) -> Option<MaskImage> {
    parse_background_image(input)
}

/// `<geometry-box>` を parse する ([`GeometryBox`] doc の grammar 参照: 7
/// keyword)。
fn parse_geometry_box(input: &mut Parser<'_, '_>) -> Option<GeometryBox> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "border-box" => Some(GeometryBox::BorderBox),
        "padding-box" => Some(GeometryBox::PaddingBox),
        "content-box" => Some(GeometryBox::ContentBox),
        "margin-box" => Some(GeometryBox::MarginBox),
        "fill-box" => Some(GeometryBox::FillBox),
        "stroke-box" => Some(GeometryBox::StrokeBox),
        "view-box" => Some(GeometryBox::ViewBox),
        _ => None,
    }
}

/// `clip-path: <clip-source> | [ <basic-shape> || <geometry-box> ] | none`
/// ([`ClipPath`] doc参照) を parse する。
///
/// `none` → `<clip-source>` (`<url>`) → `[ <basic-shape> || <geometry-box> ]`
/// の順で試す。最後の `[ … || … ]` は either order (shape before box or box
/// before shape) を許すため、両順を試す。`basic-shape` function token と
/// geometry-box ident は互いに排他的なので試行順は結果を左右しない。
fn parse_clip_path(input: &mut Parser<'_, '_>) -> Option<ClipPath> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(ClipPath::None);
    }
    // Try the basic-shape / geometry-box pair — either order, at least one.
    // First try basic-shape with optional trailing geometry-box.
    if let Ok(shape_with_box) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let shape = parse_basic_shape(i).ok_or_else(|| i.new_custom_error(()))?;
        let geometry_box = i
            .try_parse(|j| -> Result<GeometryBox, ParseError<'_, ()>> {
                parse_geometry_box(j).ok_or_else(|| j.new_custom_error(()))
            })
            .ok();
        Ok(ClipPath::BasicShape {
            shape: Box::new(shape),
            geometry_box,
        })
    }) {
        return Some(shape_with_box);
    }
    // Then try geometry-box with optional trailing basic-shape (the other order).
    if let Ok(box_with_shape) = input.try_parse(|i| -> Result<ClipPath, ParseError<'_, ()>> {
        let geometry_box = parse_geometry_box(i).ok_or_else(|| i.new_custom_error(()))?;
        if let Ok(shape) = i.try_parse(|j| -> Result<BasicShape, ParseError<'_, ()>> {
            parse_basic_shape(j).ok_or_else(|| j.new_custom_error(()))
        }) {
            Ok(ClipPath::BasicShape {
                shape: Box::new(shape),
                geometry_box: Some(geometry_box),
            })
        } else {
            Ok(ClipPath::GeometryBox(geometry_box))
        }
    }) {
        return Some(box_with_shape);
    }
    parse_url_value(input).map(ClipPath::Url)
}

/// `<basic-shape>` — `circle()` / `ellipse()` / `inset()` / `polygon()` / `path()`.
fn parse_basic_shape(input: &mut Parser<'_, '_>) -> Option<BasicShape> {
    let name = input
        .try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            let token = i.next()?.clone();
            match token {
                Token::Function(n) => Ok(n.as_ref().to_ascii_lowercase()),
                _ => Err(i.new_unexpected_token_error(token)),
            }
        })
        .ok()?;
    match name.as_str() {
        "circle" => input
            .parse_nested_block(|i| -> Result<CircleShape, ParseError<'_, ()>> {
                parse_circle_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Circle),
        "ellipse" => input
            .parse_nested_block(|i| -> Result<EllipseShape, ParseError<'_, ()>> {
                parse_ellipse_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Ellipse),
        "inset" => input
            .parse_nested_block(|i| -> Result<InsetShape, ParseError<'_, ()>> {
                parse_inset_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Inset),
        "polygon" => input
            .parse_nested_block(|i| -> Result<PolygonShape, ParseError<'_, ()>> {
                parse_polygon_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Polygon),
        "path" => input
            .parse_nested_block(|i| -> Result<PathShape, ParseError<'_, ()>> {
                parse_path_shape(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
            .map(BasicShape::Path),
        _ => None,
    }
}

fn parse_fill_rule(input: &mut Parser<'_, '_>) -> Option<FillRule> {
    let ident = input.expect_ident().ok()?.as_ref().to_ascii_lowercase();
    match ident.as_str() {
        "nonzero" => Some(FillRule::NonZero),
        "evenodd" => Some(FillRule::EvenOdd),
        _ => None,
    }
}

/// Try to parse `<shape-radius>` as `closest-side` / `farthest-side` or `<length-percentage [0,∞]>`.
fn try_parse_shape_radius(input: &mut Parser<'_, '_>) -> Option<ShapeRadius> {
    if let Ok(sr) = input.try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
        let ident = i.expect_ident()?.as_ref().to_ascii_lowercase();
        match ident.as_str() {
            "closest-side" => Ok(ShapeRadius::ClosestSide),
            "farthest-side" => Ok(ShapeRadius::FarthestSide),
            _ => Err(i.new_custom_error(())),
        }
    }) {
        return Some(sr);
    }
    let length = parse_length_value(input, true)?;
    if length_payload(length) < 0.0 || length_payload(length).is_nan() {
        return None;
    }
    Some(ShapeRadius::Length(length))
}

fn parse_circle_shape(input: &mut Parser<'_, '_>) -> Option<CircleShape> {
    let radius = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(CircleShape { radius, position })
}

fn parse_ellipse_shape(input: &mut Parser<'_, '_>) -> Option<EllipseShape> {
    let radius_x = input
        .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
            try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    let radius_y = if radius_x.is_some() {
        input
            .try_parse(|i| -> Result<ShapeRadius, ParseError<'_, ()>> {
                try_parse_shape_radius(i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
    } else {
        None
    };
    let position = if input.try_parse(|i| i.expect_ident_matching("at")).is_ok() {
        let pos = parse_position_strict(input)?;
        Some(pos)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    Some(EllipseShape {
        radius_x,
        radius_y,
        position,
    })
}

fn parse_inset_shape(input: &mut Parser<'_, '_>) -> Option<InsetShape> {
    let mut insets: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))
        }) {
            if length_payload(lp).is_nan() {
                return None;
            }
            insets.push(lp);
        } else {
            break;
        }
    }
    if insets.is_empty() {
        return None;
    }
    let border_radius = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        Some(parse_inset_border_radius(input)?)
    } else {
        None
    };
    if !input.is_exhausted() {
        return None;
    }
    let (top, right, bottom, left) = match insets.len() {
        1 => (insets[0], insets[0], insets[0], insets[0]),
        2 => (insets[0], insets[1], insets[0], insets[1]),
        3 => (insets[0], insets[1], insets[2], insets[1]),
        4 => (insets[0], insets[1], insets[2], insets[3]),
        _ => unreachable!(),
    };
    Some(InsetShape {
        top,
        right,
        bottom,
        left,
        border_radius,
    })
}

fn parse_inset_border_radius(input: &mut Parser<'_, '_>) -> Option<InsetBorderRadius> {
    let mut horiz: Vec<Length> = Vec::new();
    for _ in 0..4 {
        if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if length_payload(l) < 0.0 || length_payload(l).is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok(l)
        }) {
            horiz.push(lp);
        } else {
            break;
        }
    }
    if horiz.is_empty() {
        return None;
    }
    let vertical = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let mut vert: Vec<Length> = Vec::new();
        for _ in 0..4 {
            if let Ok(lp) = input.try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
                let l = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
                if length_payload(l) < 0.0 || length_payload(l).is_nan() {
                    return Err(i.new_custom_error(()));
                }
                Ok(l)
            }) {
                vert.push(lp);
            } else {
                break;
            }
        }
        if vert.is_empty() {
            return None;
        }
        Some(vert)
    } else {
        None
    };
    let horiz_expanded = expand_to_four(&horiz);
    let vert_expanded = vertical.as_ref().map(|v| expand_to_four(v));
    Some(InsetBorderRadius {
        horizontal: horiz_expanded,
        vertical: vert_expanded,
    })
}

fn expand_to_four(values: &[Length]) -> [Length; 4] {
    match values.len() {
        1 => [values[0]; 4],
        2 => [values[0], values[1], values[0], values[1]],
        3 => [values[0], values[1], values[2], values[1]],
        4 => [values[0], values[1], values[2], values[3]],
        _ => unreachable!(),
    }
}

fn parse_polygon_shape(input: &mut Parser<'_, '_>) -> Option<PolygonShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut fill_rule_consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        fill_rule_consumed = true;
    }
    if fill_rule_consumed {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let round = if input
        .try_parse(|i| i.expect_ident_matching("round"))
        .is_ok()
    {
        let len = parse_length_value(input, true)?;
        if length_payload(len) < 0.0 || length_payload(len).is_nan() {
            return None;
        }
        let _ = input.try_parse(|i| i.expect_comma()).ok();
        Some(len)
    } else {
        None
    };
    let mut points: Vec<(Length, Length)> = Vec::new();
    loop {
        let point = input.try_parse(|i| -> Result<(Length, Length), ParseError<'_, ()>> {
            let x = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if length_payload(x).is_nan() {
                return Err(i.new_custom_error(()));
            }
            let y = parse_length_value(i, true).ok_or_else(|| i.new_custom_error(()))?;
            if length_payload(y).is_nan() {
                return Err(i.new_custom_error(()));
            }
            Ok((x, y))
        });
        match point {
            Ok(p) => {
                points.push(p);
                let _ = input.try_parse(|i| i.expect_comma()).ok();
            }
            Err(_) => break,
        }
    }
    if points.is_empty() {
        return None;
    }
    if !input.is_exhausted() {
        return None;
    }
    Some(PolygonShape {
        fill_rule,
        round,
        points,
    })
}

fn parse_path_shape(input: &mut Parser<'_, '_>) -> Option<PathShape> {
    let mut fill_rule = FillRule::NonZero;
    let mut consumed = false;
    if let Ok(fr) = input.try_parse(|i| -> Result<FillRule, ParseError<'_, ()>> {
        parse_fill_rule(i).ok_or_else(|| i.new_custom_error(()))
    }) {
        fill_rule = fr;
        consumed = true;
    }
    if consumed {
        if input.try_parse(|i| i.expect_comma()).is_err() {
            return None;
        }
    } else {
        let _ = input.try_parse(|i| i.expect_comma()).ok();
    }
    let path_str = input.expect_string().ok()?.as_ref().to_string();
    if !input.is_exhausted() {
        return None;
    }
    Some(PathShape {
        fill_rule,
        path: path_str,
    })
}

/// `<number>` for the `transform` functions that take a bare number
/// (`matrix()`/`scale()`/`scaleX()`/`scaleY()`) — CSS Transforms Level 1
/// §9.1 places no range restriction on any of these, so — unlike
/// [`parse_filter_amount`], whose `>= 0.0` filter incidentally also
/// rejects NaN (`NaN >= 0.0` is `false` under IEEE 754) — this helper
/// cannot lean on a range check to catch the same hazard and must guard
/// explicitly. Same class of hazard as
/// [`PropertyValue::Opacity`]'s parser (`parse_opacity_value`'s
/// `!is_nan()` guard doc is canonical for the *mechanism*) — but the
/// *reason a guard is needed at all* here is `transform`-specific: this
/// crate's `Length`-typed box fields (`width`/`margin`/etc.) can go
/// unguarded because `raikiri-dom::layout::sanitize_finite` normalizes
/// NaN at the one sink that consumes them ([`Length`] doc's "型は層を
/// 表明しない" note describes that pipeline); `transform`'s
/// `f32`/[`Length`]/[`Angle`] payloads have no such downstream sink (no
/// paint-side consumer exists yet at all), so nothing else in this
/// crate's pipeline will ever normalize a NaN that slips past this
/// parser. `+Inf`/`-Inf` (ordinary magnitude overflow, a different hazard
/// class per `parse_opacity_value` doc's own distinction) are preserved —
/// no bound restricts a plain `<number>`.
///
/// `expect_number_stable` already corrects a huge-exponent, zero-mantissa
/// literal like `scale(0e999)` (module doc's "Numeric-token NaN
/// stabilization" section) before this function sees `n`, so in ordinary
/// use this `!is_nan()` check is defense-in-depth, not the primary
/// mechanism.
pub(crate) fn parse_transform_number(input: &mut Parser<'_, '_>) -> Option<f32> {
    let n = expect_number_stable(input).ok()?;
    (!n.is_nan()).then_some(n)
}

/// `<length-percentage>` for the `transform` functions that take one
/// (`translate()`/`translateX()`/`translateY()`) — same `!is_nan()`
/// rationale as [`parse_transform_number`], applied on top of
/// [`parse_length_value`]'s output instead of a bare `<number>`. Unlike
/// [`parse_non_negative_length`] (whose `>= 0.0` filter incidentally also
/// rejects NaN), `translate()`'s `<length-percentage>` has no sign
/// restriction, so there is no such incidental filter here — the guard
/// must be explicit, same shape as [`parse_transform_number`]'s own doc.
/// [`parse_length_value`] already routes through `next_numeric_stable`
/// (module doc above), so this is likewise defense-in-depth in ordinary
/// use.
pub(crate) fn parse_transform_length_percentage(input: &mut Parser<'_, '_>) -> Option<Length> {
    let length = parse_length_value(input, true)?;
    (!length_payload(length).is_nan()).then_some(length)
}

/// `[<angle> | <zero>]` for `transform`'s `rotate()`/`skew()`/`skewX()`/
/// `skewY()` and `filter`'s `hue-rotate()` (both share this exact grammar,
/// CSS Transforms Level 1 §9.1 / CSS Filter Effects Level 1 §6.1 — neither
/// restricts `<angle>`'s range; `hue-rotate()`'s own text additionally
/// states implementations "must not normalize" the value, ruling out a
/// modulo-360 transform here). Wraps [`parse_angle`] (the shared
/// gradient-facing helper, `Result`-returning) with the same `!is_nan()`
/// guard [`parse_transform_number`] applies. `parse_angle` acquires its
/// token via `next_numeric_stable` (module doc above), which corrects a
/// huge-*exponent* literal like `rotate(0e999deg)` before `parse_angle`
/// ever computes degrees from it, so — same as the guards above — this is
/// defense-in-depth rather than the primary mechanism in ordinary use.
/// Guarded here at the call site rather than inside `parse_angle` itself,
/// to avoid changing that shared helper's behavior for its other
/// (gradient) callers.
pub(crate) fn parse_angle_reject_nan(input: &mut Parser<'_, '_>) -> Option<Angle> {
    let angle = input.try_parse(parse_angle).ok()?;
    (!angle.0.is_nan()).then_some(angle)
}

fn parse_matrix_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let a = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let b = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let c = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let d = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let e = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    input.expect_comma()?;
    let f = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(TransformFunction::Matrix([a, b, c, d, e, f]))
}

/// `translate(<length-percentage>, <length-percentage>?)` — 2nd argument
/// omitted defaults to `0` (CSS Transforms Level 1 §9.1 grammar's `?`
/// multiplier on the 2nd slot; the spec text names this default
/// explicitly in the 1-argument `translate()` case).
pub(crate) fn parse_translate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let tx = parse_transform_length_percentage(input).ok_or_else(|| input.new_custom_error(()))?;
    let ty = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_length_percentage(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(TransformFunction::Translate(tx, ty))
}

fn parse_translate_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_translate_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_length_percentage(input)
        .map(TransformFunction::TranslateY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `scale(<number>, <number>?)` — 2nd argument omitted **copies the 1st**
/// (CSS Transforms Level 1 §9.1's 1-argument `scale()` text: "the second
/// value defaults to the same value as the first"), unlike `translate()`'s
/// "defaults to 0" or `skew()`'s "defaults to 0deg" — three different
/// defaulting rules across the three 2-argument functions.
pub(crate) fn parse_scale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let sx = parse_transform_number(input).ok_or_else(|| input.new_custom_error(()))?;
    let sy = input
        .try_parse(|i| -> Result<f32, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_transform_number(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(sx);
    Ok(TransformFunction::Scale(sx, sy))
}

fn parse_scale_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_scale_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_transform_number(input)
        .map(TransformFunction::ScaleY)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::Rotate)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `skew([<angle> | <zero>], [<angle> | <zero>]?)` — 2nd argument omitted
/// defaults to `0deg` (CSS Transforms Level 1 §9.1's 1-argument `skew()`
/// text), the same "defaults to 0" shape as `translate()` (unlike
/// `scale()`'s "copies the 1st" — see `parse_scale_args` doc).
pub(crate) fn parse_skew_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let ax = parse_angle_reject_nan(input).ok_or_else(|| input.new_custom_error(()))?;
    let ay = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            i.expect_comma()?;
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(TransformFunction::Skew(ax, ay))
}

fn parse_skew_x_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewX)
        .ok_or_else(|| input.new_custom_error(()))
}

fn parse_skew_y_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    parse_angle_reject_nan(input)
        .map(TransformFunction::SkewY)
        .ok_or_else(|| input.new_custom_error(()))
}

/// `<transform-function>` (CSS Transforms Level 1 §9.1、[`TransformFunction`]
/// doc参照) の 1 function を function-token 名で dispatch する
/// ([`parse_gradient`] と同じ pattern)。3D function 名
/// (`translate3d`/`rotate3d`/`matrix3d`/`perspective` 等、§10) は
/// unrecognized name として `_` arm に落ち reject する
/// ([`TransformFunction`] doc の Non-goal 節参照)。
pub(crate) fn parse_transform_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<TransformFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "matrix" => input.parse_nested_block(parse_matrix_args),
        "translate" => input.parse_nested_block(parse_translate_args),
        "translatex" => input.parse_nested_block(parse_translate_x_args),
        "translatey" => input.parse_nested_block(parse_translate_y_args),
        "scale" => input.parse_nested_block(parse_scale_args),
        "scalex" => input.parse_nested_block(parse_scale_x_args),
        "scaley" => input.parse_nested_block(parse_scale_y_args),
        "rotate" => input.parse_nested_block(parse_rotate_args),
        "skew" => input.parse_nested_block(parse_skew_args),
        "skewx" => input.parse_nested_block(parse_skew_x_args),
        "skewy" => input.parse_nested_block(parse_skew_y_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// `transform: none | <transform-list>` (CSS Transforms Level 1 §4、
/// `<transform-list> = <transform-function>[+]`
/// <https://www.w3.org/TR/css-transforms-1/#typedef-transform-list> —
/// **whitespace**-separated, not comma-separated, one or more) を parse
/// する。[`parse_text_decoration_line`]'s `||` loop と同じ「`try_parse` が
/// 失敗するまで繰り返す」shape — cssparser の `Parser::next`/`try_parse` は
/// token 間の whitespace を自動 skip するため、明示的な separator handling
/// は不要。
fn parse_transform(input: &mut Parser<'_, '_>) -> Option<Vec<TransformFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    while let Ok(function) = input.try_parse(parse_transform_function) {
        functions.push(function);
    }
    (!functions.is_empty()).then_some(functions)
}

/// `<number-percentage>` for the 7 `filter` amount functions
/// (`brightness()`/`contrast()`/`grayscale()`/`invert()`/`opacity()`/
/// `saturate()`/`sepia()`, CSS Filter Effects Level 1 §6.1). Each states
/// "Negative values are not allowed" with no opacity-property-style
/// specified/computed split (`filter`'s own Computed value is "as
/// specified", [`FilterFunction`] doc's "Range restriction is reject, not
/// clamp" section) — so this crate rejects a negative parse outright
/// (`None`), matching [`parse_nonneg_finite_number`]'s (flex-grow/
/// flex-shrink) reject-at-parse precedent rather than
/// [`parse_opacity_value`]'s preserve-then-clamp one.
///
/// No separate `!is_nan()` guard is needed: `v >= 0.0` is `false` for NaN
/// under IEEE 754 comparison semantics, so the same range check that
/// rejects an ordinary negative value would incidentally also reject a
/// NaN parse — unlike [`parse_transform_number`] (whose callers have no
/// range restriction to lean on and must guard explicitly), this helper's
/// range restriction alone would do the job. In practice a huge-exponent
/// literal like `brightness(0e999)` no longer reaches this check as NaN
/// at all — `expect_number_stable`/`expect_percentage_stable` (module
/// doc's "Numeric-token NaN stabilization" section) already correct it to
/// `0.0` at acquisition, so `v >= 0.0` accepts it normally instead of
/// incidentally rejecting it.
pub(crate) fn parse_filter_amount<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<f32, ParseError<'i, ()>> {
    let v = if let Ok(pct) = input.try_parse(|i| expect_percentage_stable(i)) {
        pct
    } else {
        expect_number_stable(input)?
    };
    if v >= 0.0 {
        Ok(v)
    } else {
        Err(input.new_custom_error(()))
    }
}

fn parse_blur_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    let length = input
        .try_parse(|i| -> Result<Length, ParseError<'_, ()>> {
            parse_non_negative_length(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Length::Px(0.0));
    Ok(FilterFunction::Blur(length))
}

fn parse_brightness_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Brightness(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_contrast_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Contrast(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_grayscale_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Grayscale(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_hue_rotate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let angle = input
        .try_parse(|i| -> Result<Angle, ParseError<'_, ()>> {
            parse_angle_reject_nan(i).ok_or_else(|| i.new_custom_error(()))
        })
        .unwrap_or(Angle(0.0));
    Ok(FilterFunction::HueRotate(angle))
}

fn parse_invert_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Invert(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_filter_opacity_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Opacity(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_saturate_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Saturate(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

fn parse_sepia_args<'i>(input: &mut Parser<'i, '_>) -> Result<FilterFunction, ParseError<'i, ()>> {
    Ok(FilterFunction::Sepia(
        input.try_parse(parse_filter_amount).unwrap_or(1.0),
    ))
}

/// `drop-shadow(<color>? && <length>{2,3})` — CSS Filter Effects Level 1
/// §6.1: "Values are interpreted as for box-shadow but with the optional
/// 3rd `<length>` value being the standard deviation instead of blur
/// radius" — grammar-identical to `text-shadow`'s own `<shadow>` syntax
/// (no spread, no inset), so [`parse_text_shadow_item`] is reused verbatim
/// ([`FilterFunction::DropShadow`] doc参照). [`parse_text_shadow_lengths`]'s
/// offset-x/offset-y already go through [`parse_shadow_length_reject_nan`]'s
/// `!is_nan()` guard (see that function's doc), so this reuse inherits the
/// guard automatically — no separate guard needed at this call site.
pub(crate) fn parse_drop_shadow_args<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let item = parse_text_shadow_item(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(FilterFunction::DropShadow(item))
}

/// `<filter-function>` (CSS Filter Effects Level 1 §6、[`FilterFunction`]
/// doc参照) の 1 function を function-token 名で dispatch する — `<url>`
/// alternative は含まない ([`parse_filter`] が別途試す、`url` という
/// function 名はここでは未知 name として reject される)。
fn parse_filter_function<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<FilterFunction, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "blur" => input.parse_nested_block(parse_blur_args),
        "brightness" => input.parse_nested_block(parse_brightness_args),
        "contrast" => input.parse_nested_block(parse_contrast_args),
        "grayscale" => input.parse_nested_block(parse_grayscale_args),
        "hue-rotate" => input.parse_nested_block(parse_hue_rotate_args),
        "invert" => input.parse_nested_block(parse_invert_args),
        "opacity" => input.parse_nested_block(parse_filter_opacity_args),
        "saturate" => input.parse_nested_block(parse_saturate_args),
        "sepia" => input.parse_nested_block(parse_sepia_args),
        "drop-shadow" => input.parse_nested_block(parse_drop_shadow_args),
        _ => Err(input.new_custom_error(())),
    }
}

/// `filter: none | <filter-value-list>` (CSS Filter Effects Level 1 §5、
/// `<filter-value-list> = [ <filter-function> | <url> ]+` —
/// **whitespace**-separated, not comma-separated, one or more) を parse
/// する。[`parse_transform`] と同じ loop shape だが、各要素で
/// [`parse_filter_function`] (named function) を先に試し、失敗したら
/// [`parse_url_value`] (`<url>` alternative) を試す 2-way fallback
/// ([`parse_background_image`] の gradient-then-url 順序と同じ理由 —
/// 互いに排他的な token shape なので試行順は結果を左右しない)。
fn parse_filter(input: &mut Parser<'_, '_>) -> Option<Vec<FilterFunction>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    loop {
        if let Ok(function) = input.try_parse(parse_filter_function) {
            functions.push(function);
            continue;
        }
        if let Ok(url) = input.try_parse(|i| -> Result<String, ParseError<'_, ()>> {
            parse_url_value(i).ok_or_else(|| i.new_custom_error(()))
        }) {
            functions.push(FilterFunction::Url(url));
            continue;
        }
        break;
    }
    (!functions.is_empty()).then_some(functions)
}

/// `<gradient>` (CSS Images 4 §3 — [`Gradient`] doc参照) の 6 function 名を
/// dispatch する。function token の名前を見てから対応する
/// `parse_*_gradient_body` を `parse_nested_block` で呼ぶ — `color-mix()`
/// 等の他 function dispatch ([`parse_color_float`] の match) と同じ形。
fn parse_gradient<'i>(input: &mut Parser<'i, '_>) -> Result<Gradient, ParseError<'i, ()>> {
    let name = match input.next()?.clone() {
        Token::Function(name) => name,
        token => return Err(input.new_unexpected_token_error(token)),
    };
    match name.as_ref().to_ascii_lowercase().as_str() {
        "linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, false))
            .map(Gradient::Linear),
        "repeating-linear-gradient" => input
            .parse_nested_block(|i| parse_linear_gradient_body(i, true))
            .map(Gradient::Linear),
        "radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, false))
            .map(Gradient::Radial),
        "repeating-radial-gradient" => input
            .parse_nested_block(|i| parse_radial_gradient_body(i, true))
            .map(Gradient::Radial),
        "conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, false))
            .map(Gradient::Conic),
        "repeating-conic-gradient" => input
            .parse_nested_block(|i| parse_conic_gradient_body(i, true))
            .map(Gradient::Conic),
        _ => Err(input.new_custom_error(())),
    }
}

/// `<angle> | <zero>` (CSS Values 4 §7.1、[`Angle`] doc参照)。bare `0` のみ
/// unitless を許す — CSS Values 4 §7.1 が明記する通り、`<angle>` 自体は
/// 一般には unitless zero を許さない ("For legacy reasons, some uses of
/// `<angle>` allow a bare 0 to mean 0deg. This is not true in general")。
/// この crate が呼び出し元 (`linear-gradient()` の `[ <angle> | <zero> | to
/// <side-or-corner> ]`、`conic-gradient()` の `from [ <angle> | <zero> ]`、
/// いずれも CSS Images 4 §3) の grammar に明示的な `<zero>` alternative を
/// 持つ「legacy な用法」に該当するため、bare `0` を受理する
/// (`<length-percentage>` の unitless-zero — [`parse_length_value`]の
/// "Unitless zero" 節 — とは別の、angle 固有の根拠)。単位変換
/// (grad/rad/turn → deg) の overflow saturation は同関数の "Percentage
/// overflow" 節と同じ方針 (`is_infinite()` の場合のみ符号を保持して
/// `f32::MAX` へ寄せる、`NaN` は無変換)。token 取得は `next_numeric_stable`
/// 経由 (module doc 冒頭「Numeric-token NaN stabilization」節参照) — 通常の
/// parse では `value` に `0e999deg` 由来の `NaN` が届くことはもう無い。
fn parse_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    match next_numeric_stable(input)? {
        Token::Number { value, .. } => {
            if value == 0.0 {
                Ok(Angle(0.0))
            } else {
                Err(input.new_custom_error(()))
            }
        }
        Token::Dimension { value, unit, .. } => {
            let degrees = match unit.to_ascii_lowercase().as_str() {
                "deg" => value,
                "grad" => value * 0.9,
                "rad" => value.to_degrees(),
                "turn" => value * 360.0,
                _ => return Err(input.new_custom_error(())),
            };
            let degrees = if degrees.is_infinite() {
                f32::MAX.copysign(degrees)
            } else {
                degrees
            };
            Ok(Angle(degrees))
        }
        token => Err(input.new_unexpected_token_error(token)),
    }
}

/// `<angle-percentage>` ([`AnglePercentage`] doc参照) — `conic-gradient()`
/// の angular color stop position。
fn parse_angle_percentage<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AnglePercentage, ParseError<'i, ()>> {
    if let Ok(percent) = input.try_parse(parse_percent_number) {
        return Ok(AnglePercentage::Percent(percent));
    }
    parse_angle(input).map(AnglePercentage::Angle)
}

/// `<percentage>` token を authored number (`50%` → `50.0`) へ変換する —
/// [`parse_length_value`] の `Token::Percentage` arm と同じ overflow
/// saturation 方針 (`unit_value * 100.0` の逆変換が `±Inf` になった場合のみ
/// 符号を保持して `f32::MAX` へ寄せる、同関数の "Percentage overflow" 節
/// 参照)。
fn parse_percent_number<'i>(input: &mut Parser<'i, '_>) -> Result<f32, ParseError<'i, ()>> {
    let unit_value = expect_percentage_stable(input)?;
    let percent = unit_value * 100.0;
    Ok(if percent.is_infinite() {
        f32::MAX.copysign(percent)
    } else {
        percent
    })
}

/// `<length-percentage [0,∞]>` — [`parse_length_percentage_res`] に
/// non-negative filter ([`length_payload`]の doc の `[0,∞]` pattern) を足した
/// もの。`radial-gradient()`のellipse 2-radii form
/// (`<length-percentage [0,∞]>{2}`、[`RadialSize::Ellipse`]) が使う。
fn parse_non_negative_length_percentage_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Length, ParseError<'i, ()>> {
    let length = parse_length_value(input, true).ok_or_else(|| input.new_custom_error(()))?;
    if length_payload(length) >= 0.0 {
        Ok(length)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// `in <color-space> <hue-interpolation-method>?` ([`GradientColorInterpolation`]
/// doc参照) — parse + validation は `parse_color_mix_function`と共有する
/// [`parse_color_interpolation_method`] に委譲し、ここでは結果 tuple を
/// [`GradientColorInterpolation`] へ組み立てるだけ。
pub(crate) fn parse_gradient_color_interpolation<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorInterpolation, ParseError<'i, ()>> {
    let (color_space, hue_method) = parse_color_interpolation_method(input, false)?;
    Ok(GradientColorInterpolation {
        color_space,
        hue_method,
    })
}

/// [`GradientColorInterpolation`] の spec-mandated default — `in ...` 節
/// 省略時、CSS Images 4 §3.5.2 "Coloring the Gradient Line" "the color
/// space used for gradient interpolation is the default interpolation
/// color space, Oklab"。
fn default_gradient_color_interpolation() -> GradientColorInterpolation {
    GradientColorInterpolation {
        color_space: MixColorSpace::Oklab,
        hue_method: HueInterpolationMethod::Shorter,
    }
}

/// `at <position>` ([`CssPosition`] doc参照) — `radial-gradient()` /
/// `conic-gradient()` 共通。
fn parse_at_position<'i>(input: &mut Parser<'i, '_>) -> Result<CssPosition, ParseError<'i, ()>> {
    input.expect_ident_matching("at")?;
    parse_bg_position(input).ok_or_else(|| input.new_custom_error(()))
}

/// `<position>` の spec-mandated default (`center`、[`css_position_center`]
/// を両軸に適用)。
fn default_center_position() -> CssPosition {
    CssPosition {
        horizontal: css_position_center(),
        vertical: css_position_center(),
    }
}

/// `<color>` を持つ gradient stop の color ([`GradientStopColor`] doc参照)
/// — `currentcolor` keyword を先取りしてから [`parse_color`] へ委譲する
/// ([`parse_text_shadow_color`] と同じ shape)。
fn parse_gradient_stop_color(input: &mut Parser<'_, '_>) -> Option<GradientStopColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(GradientStopColor::CurrentColor);
    }
    parse_color(input).map(GradientStopColor::Resolved)
}

/// `<linear-color-stop> = <color> <length-percentage>?`
/// ([`GradientColorStop`] doc参照)。
fn parse_gradient_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<GradientColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_length_percentage_res).ok();
    Ok(GradientColorStop { color, position })
}

/// `<color-stop-list>` ([`GradientColorStop`] doc の scope carving 節 —
/// hint 無し、2 個以上必須)。`linear-gradient()`/`radial-gradient()` 共通。
fn parse_gradient_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<GradientColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_gradient_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// `<angular-color-stop> = <color> <color-stop-angle>?` の 1-value 版
/// ([`AngularColorStop`] doc参照)。
fn parse_angular_color_stop<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<AngularColorStop, ParseError<'i, ()>> {
    let color = parse_gradient_stop_color(input).ok_or_else(|| input.new_custom_error(()))?;
    let position = input.try_parse(parse_angle_percentage).ok();
    Ok(AngularColorStop { color, position })
}

/// `<angular-color-stop-list>` — [`parse_gradient_color_stop_list`]の
/// conic 版 (同じ scope carving、同じ 2 個以上 minimum)。
fn parse_angular_color_stop_list<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Vec<AngularColorStop>, ParseError<'i, ()>> {
    let stops = input.parse_comma_separated(parse_angular_color_stop)?;
    if stops.len() < 2 {
        return Err(input.new_custom_error(()));
    }
    Ok(stops)
}

/// [`LinearGradientDirection`]の spec-mandated default (`to bottom`、CSS
/// Images 4 §3.1 "If the first argument to the function is omitted, it
/// defaults to to bottom")。
fn default_linear_gradient_direction() -> LinearGradientDirection {
    LinearGradientDirection::Side(SideOrCorner {
        horizontal: None,
        vertical: Some(VerticalSide::Bottom),
    })
}

/// `to <side-or-corner>` の `to` 抜きの部分、または `<angle>` — どちらか
/// ([`LinearGradientDirection`] doc の 2 alternative)。
fn parse_linear_gradient_direction<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LinearGradientDirection, ParseError<'i, ()>> {
    if input.try_parse(|i| i.expect_ident_matching("to")).is_ok() {
        return parse_side_or_corner(input).map(LinearGradientDirection::Side);
    }
    parse_angle(input).map(LinearGradientDirection::Angle)
}

/// `<side-or-corner> = [left | right] || [top | bottom]`
/// ([`SideOrCorner`] doc参照) — [`parse_outline`]等と同じ any-order loop。
fn parse_side_or_corner<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<SideOrCorner, ParseError<'i, ()>> {
    let mut horizontal: Option<HorizontalSide> = None;
    let mut vertical: Option<VerticalSide> = None;
    loop {
        if horizontal.is_none()
            && let Ok(value) = input.try_parse(parse_horizontal_side)
        {
            horizontal = Some(value);
            continue;
        }
        if vertical.is_none()
            && let Ok(value) = input.try_parse(parse_vertical_side)
        {
            vertical = Some(value);
            continue;
        }
        break;
    }
    if horizontal.is_none() && vertical.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok(SideOrCorner {
        horizontal,
        vertical,
    })
}

fn parse_horizontal_side<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<HorizontalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "left" => Ok(HorizontalSide::Left),
        "right" => Ok(HorizontalSide::Right),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_vertical_side<'i>(input: &mut Parser<'i, '_>) -> Result<VerticalSide, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "top" => Ok(VerticalSide::Top),
        "bottom" => Ok(VerticalSide::Bottom),
        _ => Err(input.new_custom_error(())),
    }
}

/// `linear-gradient()`/`repeating-linear-gradient()` の nested block body
/// ([`LinearGradient`] doc の grammar 参照)。`direction`/`interpolation` は
/// `||` (any order, both optional) — 見つかった場合のみ、続く
/// `<color-stop-list>` の前に comma が要る (grammar の trailing `,` は
/// `[...]?` group の**外**にあるので、group が空なら comma も現れない —
/// `linear-gradient(red, blue)` に direction/interpolation が無いのと同じ
/// 理由)。
fn parse_linear_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<LinearGradient, ParseError<'i, ()>> {
    let mut direction: Option<LinearGradientDirection> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if direction.is_none()
            && let Ok(value) = input.try_parse(parse_linear_gradient_direction)
        {
            direction = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    if direction.is_some() || interpolation.is_some() {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    Ok(LinearGradient {
        repeating,
        direction: direction.unwrap_or_else(default_linear_gradient_direction),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_radial_shape_keyword<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShape, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "circle" => Ok(RadialShape::Circle),
        "ellipse" => Ok(RadialShape::Ellipse),
        _ => Err(input.new_custom_error(())),
    }
}

fn parse_radial_extent<'i>(input: &mut Parser<'i, '_>) -> Result<RadialExtent, ParseError<'i, ()>> {
    let ident = input.expect_ident()?.clone();
    match ident.as_ref().to_ascii_lowercase().as_str() {
        "closest-side" => Ok(RadialExtent::ClosestSide),
        "closest-corner" => Ok(RadialExtent::ClosestCorner),
        "farthest-side" => Ok(RadialExtent::FarthestSide),
        "farthest-corner" => Ok(RadialExtent::FarthestCorner),
        _ => Err(input.new_custom_error(())),
    }
}

/// [`RadialSize`]の authored form — [`resolve_radial_shape_and_size`] が
/// (省略された `shape` と合わせて) 最終的な `(RadialShape, RadialSize)` へ
/// 解決する前の中間表現。`Circle`/`Ellipse` は [`RadialSize`]と同じ意味だが
/// まだ `shape` との整合性を確認していない。
enum RadialSizeAuthored {
    Extent(RadialExtent),
    Circle(Length),
    Ellipse(Length, Length),
}

/// [`parse_radial_shape_size_position_group`] の戻り値 shape — clippy
/// `type_complexity` を避けるための alias (意味論的な新型ではない)。
type RadialShapeSizePositionGroup = (
    Option<RadialShape>,
    Option<RadialSizeAuthored>,
    Option<CssPosition>,
);

/// `<radial-size>` (CSS Images 3 §3.2.1 baseline grammar、[`RadialSize`]
/// doc参照)。2 つの `<length-percentage [0,∞]>` (ellipse form) を先に試す —
/// 単一 token しか無い入力では 2 個目の parse が失敗して丸ごと rewind し、
/// 単一 `<length [0,∞]>` (circle form) へ自然に fall back する
/// ([`parse_bg_position`]の alternative 順序 doc と同じ「安全な rewind」
/// 構造)。
fn parse_radial_size<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialSizeAuthored, ParseError<'i, ()>> {
    if let Ok(extent) = input.try_parse(parse_radial_extent) {
        return Ok(RadialSizeAuthored::Extent(extent));
    }
    if let Ok((a, b)) = input.try_parse(parse_two_non_negative_length_percentages) {
        return Ok(RadialSizeAuthored::Ellipse(a, b));
    }
    let length = parse_non_negative_length(input).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialSizeAuthored::Circle(length))
}

fn parse_two_non_negative_length_percentages<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length), ParseError<'i, ()>> {
    let a = parse_non_negative_length_percentage_res(input)?;
    let b = parse_non_negative_length_percentage_res(input)?;
    Ok((a, b))
}

/// 省略された `shape`/`size` を CSS Images 3 §3.2.1 の規則で解決する —
/// [`RadialShape`]/[`RadialSize`]のペア doc参照。無効な組み合わせ
/// (`circle` + ellipse-only size、`ellipse` + circle-only size) は `None`。
fn resolve_radial_shape_and_size(
    shape: Option<RadialShape>,
    size: Option<RadialSizeAuthored>,
) -> Option<(RadialShape, RadialSize)> {
    match (shape, size) {
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Ellipse(_, _)))
        | (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Circle(_))) => None,
        (Some(RadialShape::Circle), Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (Some(RadialShape::Ellipse), Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (Some(shape), Some(RadialSizeAuthored::Extent(extent))) => {
            Some((shape, RadialSize::Extent(extent)))
        }
        (Some(shape), None) => Some((shape, RadialSize::Extent(RadialExtent::FarthestCorner))),
        (None, Some(RadialSizeAuthored::Circle(l))) => {
            Some((RadialShape::Circle, RadialSize::Circle(l)))
        }
        (None, Some(RadialSizeAuthored::Ellipse(a, b))) => {
            Some((RadialShape::Ellipse, RadialSize::Ellipse(a, b)))
        }
        (None, Some(RadialSizeAuthored::Extent(extent))) => {
            Some((RadialShape::Ellipse, RadialSize::Extent(extent)))
        }
        (None, None) => Some((
            RadialShape::Ellipse,
            RadialSize::Extent(RadialExtent::FarthestCorner),
        )),
    }
}

/// `[ <radial-shape> || <radial-size> ]? [ at <position> ]?` — spec の
/// juxtaposition (space 区切り) が示す通り、`shape`/`size` は互いに
/// any-order (内側 `||`) だが、`at <position>` はこのグループ全体の
/// **後**にしか現れない (順序固定)。`shape`/`size`/`position` のいずれも
/// 無ければ `Err` を返す — [`parse_radial_gradient_body`]側の outer `||`
/// loop が「このグループを 1 要素として `<color-interpolation-method>`
/// と任意順に読む」ために、空マッチと「何も無かった」を区別する必要が
/// あるため (`try_parse` が空マッチを毎回成功として返すと、outer loop の
/// 2 巡目以降でこのグループを再試行できなくなる)。
fn parse_radial_shape_size_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<RadialShapeSizePositionGroup, ParseError<'i, ()>> {
    let mut shape: Option<RadialShape> = None;
    let mut size: Option<RadialSizeAuthored> = None;
    loop {
        if shape.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_keyword)
        {
            shape = Some(value);
            continue;
        }
        if size.is_none()
            && let Ok(value) = input.try_parse(parse_radial_size)
        {
            size = Some(value);
            continue;
        }
        break;
    }
    let position = input.try_parse(parse_at_position).ok();
    if shape.is_none() && size.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((shape, size, position))
}

/// `radial-gradient()`/`repeating-radial-gradient()` の nested block body
/// ([`RadialGradient`] doc の grammar 参照)。Grammar `[ [ [ <radial-shape>
/// || <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method>
/// ]?` の外側 `||` (shape/size/position グループと
/// `<color-interpolation-method>` の間) を any-order loop で読む —
/// グループ内部の順序制約 ([`parse_radial_shape_size_position_group`]
/// doc参照) は [`parse_linear_gradient_body`]の 2-slot 構造と異なりグループ
/// 自体が 1 slot になっている点に注意 (`shape`/`size`/`position` を outer
/// loop の独立した slot にすると `at center circle` のような spec-invalid
/// な順序 — position が shape/size より前 — まで受理してしまう)。
fn parse_radial_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<RadialGradient, ParseError<'i, ()>> {
    let mut group: Option<RadialShapeSizePositionGroup> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_radial_shape_size_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_gradient_color_stop_list(input)?;
    let (shape, size, position) = group.unwrap_or((None, None, None));
    let (resolved_shape, resolved_size) =
        resolve_radial_shape_and_size(shape, size).ok_or_else(|| input.new_custom_error(()))?;
    Ok(RadialGradient {
        repeating,
        shape: resolved_shape,
        size: resolved_size,
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

fn parse_conic_from_angle<'i>(input: &mut Parser<'i, '_>) -> Result<Angle, ParseError<'i, ()>> {
    input.expect_ident_matching("from")?;
    parse_angle(input)
}

/// `[ from [ <angle> | <zero> ] ]? [ at <position> ]?` —
/// [`parse_radial_shape_size_position_group`]の conic 版。spec の
/// juxtaposition により `from <angle>` は `at <position>` より必ず先
/// (こちらは shape/size 側と違い、先頭要素自体が単一 component なので
/// 内側に `||` は無い)。空マッチと区別するための `Err` fallback も同型。
fn parse_conic_from_position_group<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Option<Angle>, Option<CssPosition>), ParseError<'i, ()>> {
    let angle = input.try_parse(parse_conic_from_angle).ok();
    let position = input.try_parse(parse_at_position).ok();
    if angle.is_none() && position.is_none() {
        return Err(input.new_custom_error(()));
    }
    Ok((angle, position))
}

/// `conic-gradient()`/`repeating-conic-gradient()` の nested block body
/// ([`ConicGradient`] doc の grammar 参照)。外側 `||` (`from`/`at`
/// グループと `<color-interpolation-method>` の間) を any-order loop で
/// 読む — [`parse_radial_gradient_body`]と同じ「グループを 1 slot として
/// 扱う」構造 (`at center from 45deg` のような spec-invalid な逆順を
/// 拒否するのに必要、[`parse_conic_from_position_group`] doc参照)。
fn parse_conic_gradient_body<'i>(
    input: &mut Parser<'i, '_>,
    repeating: bool,
) -> Result<ConicGradient, ParseError<'i, ()>> {
    let mut group: Option<(Option<Angle>, Option<CssPosition>)> = None;
    let mut interpolation: Option<GradientColorInterpolation> = None;
    loop {
        if group.is_none()
            && let Ok(value) = input.try_parse(parse_conic_from_position_group)
        {
            group = Some(value);
            continue;
        }
        if interpolation.is_none()
            && let Ok(value) = input.try_parse(parse_gradient_color_interpolation)
        {
            interpolation = Some(value);
            continue;
        }
        break;
    }
    let any = group.is_some() || interpolation.is_some();
    if any {
        input.expect_comma()?;
    }
    let stops = parse_angular_color_stop_list(input)?;
    let (angle, position) = group.unwrap_or((None, None));
    Ok(ConicGradient {
        repeating,
        angle: angle.unwrap_or(Angle(0.0)),
        position: position.unwrap_or_else(default_center_position),
        interpolation: interpolation.unwrap_or_else(default_gradient_color_interpolation),
        stops: Arc::new(stops),
    })
}

/// `<repeat-style>` の 1 keyword を parse する ([`BackgroundRepeatKeyword`]
/// doc の grammar 参照)。`repeat-x`/`repeat-y` はここでは扱わない
/// ([`parse_background_repeat`] が別途 top-level alternative として処理)。
fn parse_background_repeat_keyword(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeatKeyword> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "repeat" => Some(BackgroundRepeatKeyword::Repeat),
        "space" => Some(BackgroundRepeatKeyword::Space),
        "round" => Some(BackgroundRepeatKeyword::Round),
        "no-repeat" => Some(BackgroundRepeatKeyword::NoRepeat),
        _ => None,
    }
}

fn parse_background_repeat_keyword_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<BackgroundRepeatKeyword, ParseError<'i, ()>> {
    parse_background_repeat_keyword(input).ok_or_else(|| input.new_custom_error(()))
}

/// `background-repeat: <repeat-style>` を parse する ([`BackgroundRepeat`]
/// doc の grammar 参照: `repeat-x | repeat-y | [repeat | space | round |
/// no-repeat]{1,2}`)。
///
/// `repeat-x`/`repeat-y` は 2-keyword form の shorthand として先に試す
/// (spec computed value: `repeat-x` = `repeat no-repeat`、`repeat-y` =
/// `no-repeat repeat`)。1 keyword のみ指定時は両軸に同じ値を適用する
/// (`repeat` = `repeat repeat` 等)。
pub(crate) fn parse_background_repeat(input: &mut Parser<'_, '_>) -> Option<BackgroundRepeat> {
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-x"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        });
    }
    if input
        .try_parse(|i| i.expect_ident_matching("repeat-y"))
        .is_ok()
    {
        return Some(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::Repeat,
        });
    }
    let first = parse_background_repeat_keyword(input)?;
    let second = input.try_parse(parse_background_repeat_keyword_res).ok();
    Some(match second {
        None => BackgroundRepeat { x: first, y: first },
        Some(second) => BackgroundRepeat {
            x: first,
            y: second,
        },
    })
}

/// `background-attachment: <attachment>` を parse する
/// ([`BackgroundAttachment`] doc の grammar 参照: `scroll | fixed | local`)。
fn parse_background_attachment(input: &mut Parser<'_, '_>) -> Option<BackgroundAttachment> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "scroll" => Some(BackgroundAttachment::Scroll),
        "fixed" => Some(BackgroundAttachment::Fixed),
        "local" => Some(BackgroundAttachment::Local),
        _ => None,
    }
}

/// `object-fit: <fit>` を parse する ([`ObjectFit`] doc の grammar 参照:
/// `fill | contain | cover | none | scale-down`)。
fn parse_object_fit(input: &mut Parser<'_, '_>) -> Option<ObjectFit> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "fill" => Some(ObjectFit::Fill),
        "contain" => Some(ObjectFit::Contain),
        "cover" => Some(ObjectFit::Cover),
        "none" => Some(ObjectFit::None),
        "scale-down" => Some(ObjectFit::ScaleDown),
        _ => None,
    }
}

/// `isolation: <isolation-mode>` を parse する ([`Isolation`] doc の grammar
/// 参照: `auto | isolate`)。
fn parse_isolation(input: &mut Parser<'_, '_>) -> Option<Isolation> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "auto" => Some(Isolation::Auto),
        "isolate" => Some(Isolation::Isolate),
        _ => None,
    }
}

/// `mix-blend-mode: <blend-mode>` を parse する ([`MixBlendMode`] doc の
/// grammar 参照: 16 keyword)。
fn parse_mix_blend_mode(input: &mut Parser<'_, '_>) -> Option<MixBlendMode> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "normal" => Some(MixBlendMode::Normal),
        "multiply" => Some(MixBlendMode::Multiply),
        "screen" => Some(MixBlendMode::Screen),
        "overlay" => Some(MixBlendMode::Overlay),
        "darken" => Some(MixBlendMode::Darken),
        "lighten" => Some(MixBlendMode::Lighten),
        "color-dodge" => Some(MixBlendMode::ColorDodge),
        "color-burn" => Some(MixBlendMode::ColorBurn),
        "hard-light" => Some(MixBlendMode::HardLight),
        "soft-light" => Some(MixBlendMode::SoftLight),
        "difference" => Some(MixBlendMode::Difference),
        "exclusion" => Some(MixBlendMode::Exclusion),
        "hue" => Some(MixBlendMode::Hue),
        "saturation" => Some(MixBlendMode::Saturation),
        "color" => Some(MixBlendMode::Color),
        "luminosity" => Some(MixBlendMode::Luminosity),
        _ => None,
    }
}

/// `<visual-box>` を parse する ([`VisualBox`] doc の grammar 参照:
/// `border-box | padding-box | content-box`)。`background-clip` /
/// `background-origin` 共有 (initial value の違いは呼び出し元ではなく
/// `specified.rs`/`computed.rs` 側で扱う)。
fn parse_visual_box(input: &mut Parser<'_, '_>) -> Option<VisualBox> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "border-box" => Some(VisualBox::BorderBox),
        "padding-box" => Some(VisualBox::PaddingBox),
        "content-box" => Some(VisualBox::ContentBox),
        "border-area" => Some(VisualBox::BorderArea),
        "text" => Some(VisualBox::Text),
        _ => None,
    }
}

/// `<bg-size>` の 1 軸分 — `<length-percentage [0,∞]> | auto`
/// ([`parse_width`] と同じ non-negative enforcement pattern)。
fn parse_background_size_axis(input: &mut Parser<'_, '_>) -> Option<LengthOrAuto> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Some(LengthOrAuto::Auto);
    }
    let length = parse_length_value(input, true)?;
    (length_payload(length) >= 0.0).then_some(LengthOrAuto::Length(length))
}

fn parse_background_size_axis_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<LengthOrAuto, ParseError<'i, ()>> {
    parse_background_size_axis(input).ok_or_else(|| input.new_custom_error(()))
}

/// `background-size: <bg-size>` を parse する ([`BackgroundSize`] doc の
/// grammar 参照: `[ <length-percentage [0,∞]> | auto ]{1,2} | cover |
/// contain`)。
///
/// `cover`/`contain` は keyword 全体を占有するため axis run より先に試す。
/// 2 個目の axis が省略された場合は **`auto`** (spec verbatim: "If only
/// one value is given the second is assumed to be auto.") — 1 個目の
/// 値を複製する [`parse_border_radius`] 系の fill 規則とは異なるので
/// 流用しない。
pub(crate) fn parse_background_size(input: &mut Parser<'_, '_>) -> Option<BackgroundSize> {
    if input
        .try_parse(|i| i.expect_ident_matching("cover"))
        .is_ok()
    {
        return Some(BackgroundSize::Cover);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("contain"))
        .is_ok()
    {
        return Some(BackgroundSize::Contain);
    }
    let width = parse_background_size_axis(input)?;
    let height = input
        .try_parse(parse_background_size_axis_res)
        .unwrap_or(LengthOrAuto::Auto);
    Some(BackgroundSize::Explicit { width, height })
}

/// `<bg-position> [ / <bg-size> ]?` — the `background` shorthand's
/// position+size unit ([`BackgroundShorthand`] doc's grammar section).
/// Unlike the shorthand's other `||` components, position and size are
/// **not** independently orderable — a size may only follow a position,
/// separated by a literal `/` (same fixed-pair shape as
/// [`parse_grid_line_shorthand`]'s `<grid-line> [ / <grid-line> ]?`).
///
/// [`parse_bg_position`]'s 3 internal alternatives (`branch3`/`branch2`/
/// `branch1`, see that function's doc) each consume exactly their own
/// production and stop — they never overrun into tokens that belong to a
/// later shorthand component. That is what lets this function's caller
/// ([`parse_background_shorthand`]) safely hand any leftover tokens back to
/// its own any-order loop instead of treating them as part of the position.
pub(crate) fn parse_background_position_and_size(
    input: &mut Parser<'_, '_>,
) -> Option<(CssPosition, Option<BackgroundSize>)> {
    let position = parse_bg_position(input)?;
    let size = input
        .try_parse(|i| -> Result<BackgroundSize, ParseError<'_, ()>> {
            i.expect_delim('/')?;
            parse_background_size(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok();
    Some((position, size))
}

/// `font` shorthand の `font-size` 成分 — [`parse_font_size`] の返す
/// 2 通り ([`PropertyValue::FontSize`] / [`PropertyValue::FontSizeRelative`])
/// をそのまま運ぶ small enum。[`FontShorthand`] の payload 専用で、
/// [`ComputedValues`] / [`SpecifiedValues`] には入らず、umbrella crate からも
/// re-export しない ([`BackgroundShorthand`] と同じ扱い)。
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontShorthandSize {
    /// `<length-percentage>` / absolute-size keyword — [`parse_font_size`] の
    /// [`PropertyValue::FontSize`] 側。展開先は同 variant。
    Absolute(Length),
    /// `larger` / `smaller` — [`parse_font_size`] の
    /// [`PropertyValue::FontSizeRelative`] 側。展開先は同 variant
    /// (key はどちらも [`PropertyKey::FontSize`])。
    Relative(RelativeFontSize),
}

/// `font` shorthand の specified value carrier — CSS Fonts 4 §2.1 "Font
/// shorthand: the font property"
/// (<https://www.w3.org/TR/css-fonts-4/#font-prop>)。
///
/// Spec grammar (full): `[ [ <'font-style'> || <font-variant-css2> ||
/// <'font-weight'> || <font-width-css3> ]? <'font-size'> [ / <'line-height'> ]?
/// <'font-family'># ] | <system-font>`。本実装の subset:
///
/// # Scope carving
///
/// - **Preface** (`||` 3 slot): `font-style` は [`parse_font_style`] の範囲
///   (`normal` / `italic` / bare `oblique`)、`font-weight` は
///   [`parse_font_weight`] の全範囲 (`normal` / `bold` / `bolder` /
///   `lighter` / `<number [1,1000]>`) を受理。`font-variant-css2`
///   (`normal` / `small-caps`) は [`FontVariantCaps::Normal`] /
///   [`FontVariantCaps::SmallCaps`] に畳む — CSS Fonts 3 §6.9 の `font-variant`
///   shorthand 全体ではなく CSS2 subset のみ対応 (本 crate が持つのは
///   `font-variant-caps` longhand だけで、他 sub-property が無いため)。
///   `font-width-css3` (`font-stretch`) longhand は本 crate に存在しないため
///   `normal` のみ consume して捨てる (initial と同じ値なので reset 効果は
///   observable ではない)。`normal` 以外の stretch keyword
///   (`condensed` 等)・`font-variant-css2` 外の variant 指定は preface の
///   どの slot にも match せず、後続の `font-size` parse が失敗するため
///   declaration 全体が drop される (spec-valid だが未対応 = silent drop、
///   本 crate の一般 policy)。
/// - **System fonts** (`caption` / `icon` / `menu` / `message-box` /
///   `small-caption` / `status-bar`) は受理しない — longhand への分解が
///   UA 依存で本 crate の font model に載らないため、declaration ごと drop。
/// - **`font-size`** は [`parse_font_size`] をそのまま使う (absolute-size
///   keyword / `larger` / `smaller` / `<length-percentage>` 全範囲)。
/// - **`line-height`** (`/ ...` 付きの場合のみ) は [`parse_line_height`] を
///   そのまま使う (`normal` / `<number>` / `<length-percentage>` 全範囲)。
/// - **`font-family`** は [`parse_font_family`] をそのまま使う
///   (`<family-name>#`、1 要素以上必須)。
///
/// # Initial value fill (omitted components)
///
/// CSS Fonts 4 §2.1 verbatim: "The 'font' property is a shorthand for
/// [font-style, font-variant, font-weight, font-size, line-height,
/// font-family]" — 省略成分は spec initial value で埋める
/// ([`BackgroundShorthand`] doc の同名節と同じ規則)。
/// [`parse_font_shorthand`] は省略された `style` → [`FontStyle::Normal`]、
/// `variant` → [`FontVariantCaps::Normal`]、 `weight` → `400`、
/// `line-height` → [`LineHeight::Normal`] で埋める (`size` と `family` は
/// 必須のため省略不可)。埋め値は各 standalone longhand の initial と同一
/// ([`crate::specified::SpecifiedValues::initial`] の対応 field が canonical)。
///
/// [`crate::rule::expand_shorthand_into`] が [`PropertyValue::Font`] を
/// [`PropertyValue::FontStyle`] / [`PropertyValue::FontVariantCaps`] /
/// [`PropertyValue::FontWeight`] / size ([`PropertyValue::FontSize`] /
/// [`PropertyValue::FontSizeRelative`]) / [`PropertyValue::LineHeight`] /
/// [`PropertyValue::FontFamily`] の 6 longhand に展開する —
/// margin/padding/border/outline shorthand precedent と同じ
/// "parse-time expansion, never reaches cascade" 設計 (詳細は同関数の doc)。
///
/// `#[non_exhaustive]` は付けない — sibling shorthand-only carrier
/// ([`GridLineShorthand`] / [`TextDecorationShorthand`] /
/// [`BackgroundShorthand`]) と同じ理由 (本型は [`PropertyValue::Font`] の
/// payload 専用で、[`ComputedValues`] / [`SpecifiedValues`] には入らず、
/// umbrella crate からも re-export しない)。
///
/// [`ComputedValues`]: crate::computed::ComputedValues
/// [`SpecifiedValues`]: crate::specified::SpecifiedValues
#[derive(Clone, Debug, PartialEq)]
pub struct FontShorthand {
    /// `font-style` 成分 — 省略時は [`FontStyle::Normal`] (spec initial)。
    pub style: FontStyle,
    /// `font-variant-css2` 成分 — 省略時は [`FontVariantCaps::Normal`]
    /// (spec initial)。`small-caps` は [`FontVariantCaps::SmallCaps`]。
    pub variant: FontVariantCaps,
    /// `font-weight` 成分 — 省略時は `400` (`normal`、spec initial)。
    pub weight: FontWeightValue,
    /// `font-size` 成分 (必須) — [`FontShorthandSize`] 参照。
    pub size: FontShorthandSize,
    /// `line-height` 成分 — 省略時は [`LineHeight::Normal`] (spec initial)。
    pub line_height: LineHeight,
    /// `font-family` 成分 (必須) — [`parse_font_family`] の結果を共有する
    /// `Arc` ([`PropertyValue::FontFamily`] と同じ DoS 対策 pattern)。
    pub family: Arc<Vec<Atom>>,
}

/// `font` shorthand を parse する ([`FontShorthand`] doc の grammar 節参照)。
///
/// [`parse_background_shorthand`] と同じ loop 構造: preface の各 unfilled
/// slot を `try_parse` で順に試し、成功したら slot を埋めて loop 先頭に戻る。
/// preface が確定したら必須の `font-size`、任意の `/ line-height`、必須の
/// `font-family` を順に parse する。`font-size` / `font-family` のいずれかが
/// 無い場合は `None` (declaration 全体が drop される)。leftover は呼び出し元
/// (`rule.rs` の declaration parser) の `expect_exhausted` が丸ごと drop する
/// ([`parse_background_shorthand`] と同じ契約)。
fn parse_font_shorthand(input: &mut Parser<'_, '_>) -> Option<FontShorthand> {
    // System-font keyword は longhand に分解できないため先に reject
    // (`font: menu` 等が preface の `normal` 扱いで誤って受理されるのを防ぐ)。
    // `try_parse` で囲むため cursor は消費されない。
    let is_system_font = [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ]
    .iter()
    .any(|keyword| {
        input
            .try_parse(|i| i.expect_ident_matching(keyword))
            .is_ok()
    });
    if is_system_font {
        return None;
    }

    let mut style: Option<FontStyle> = None;
    let mut variant: Option<FontVariantCaps> = None;
    let mut weight: Option<FontWeightValue> = None;
    // `font-stretch` longhand は本 crate に無いため `normal` のみ consume
    // して捨てる (`FontShorthand` doc の Scope carving 節参照)。
    let mut stretch_seen = false;

    loop {
        if style.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontStyle, ParseError<'_, ()>> {
                parse_font_style(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            style = Some(v);
            continue;
        }
        if variant.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontVariantCaps, ParseError<'_, ()>> {
                if i.try_parse(|j| j.expect_ident_matching("normal")).is_ok() {
                    Ok(FontVariantCaps::Normal)
                } else if i
                    .try_parse(|j| j.expect_ident_matching("small-caps"))
                    .is_ok()
                {
                    Ok(FontVariantCaps::SmallCaps)
                } else {
                    Err(i.new_custom_error(()))
                }
            })
        {
            variant = Some(v);
            continue;
        }
        if weight.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<FontWeightValue, ParseError<'_, ()>> {
                parse_font_weight(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            weight = Some(v);
            continue;
        }
        if !stretch_seen
            && input
                .try_parse(|i| i.expect_ident_matching("normal"))
                .is_ok()
        {
            stretch_seen = true;
            continue;
        }
        break;
    }

    // 必須の `font-size` (`parse_font_size` が `PropertyValue` を返すため
    // `FontShorthandSize` に畳む — 同関数はこの 2 variant しか返さない)。
    let size = match parse_font_size(input)? {
        PropertyValue::FontSize(length) => FontShorthandSize::Absolute(length),
        PropertyValue::FontSizeRelative(relative) => FontShorthandSize::Relative(relative),
        // cov:ignore: `parse_font_size` は上記 2 variant しか返さない
        // (同関数の 2 return path が canonical) — 到達不能。
        _ => return None,
    };

    // 任意の `/ line-height`。
    let line_height = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(input)?
    } else {
        LineHeight::Normal
    };

    // 必須の `font-family` (`<family-name>#`、1 要素以上)。
    let family = parse_font_family(input)?;

    Some(FontShorthand {
        style: style.unwrap_or(FontStyle::Normal),
        variant: variant.unwrap_or(FontVariantCaps::Normal),
        weight: weight.unwrap_or(FontWeightValue::Absolute(400.0)),
        size,
        line_height,
        family: Arc::new(family),
    })
}

/// `background` shorthand を parse する ([`BackgroundShorthand`] doc の
/// grammar 節参照)。
///
/// # `||` (any-order) grammar semantics
///
/// [`parse_border_shorthand`] と同じ loop 構造: 各 unfilled slot を
/// `try_parse` で順に試し、成功したら slot を埋めて loop 先頭に戻る。全 slot
/// 満了、または未 match token に当たったら break — leftover は呼び出し元
/// (`rule.rs` の declaration parser) の `expect_exhausted` が丸ごと drop する
/// ([`BackgroundShorthand`] doc の「単一 layer のみ対応」節が、これを使って
/// comma-separated 複数 layer を reject する仕組みを説明している)。
///
/// 6 slot のうち `visual_boxes` だけ最大 2 回一致しうる — `<visual-box>` が
/// grammar 上 2 回独立した `||` alternative として現れるため
/// ([`BackgroundShorthand`] doc 参照)。`position_and_size` は
/// [`parse_background_position_and_size`] 経由で 1 slot として扱う (spec の
/// `<bg-position> [ / <bg-size> ]?` を単一の `||` alternative として)。
///
/// 各 slot の token 集合は互いに素 (image は `none`/`url()`/gradient
/// function、position は方向 keyword/length、repeat-style/attachment/
/// visual-box はそれぞれ固有 keyword 集合、color は named color/hex/function)
/// なので、slot を試す順序自体は結果を左右しない —
/// [`parse_border_shorthand`] doc の同旨コメント参照。
///
/// # At least 1 component required
///
/// spec CSS Values 4 §2.2 `||` semantics: "one or more of them must occur,
/// in any order." — 0 component (空 `background:` や未知 keyword のみ) は
/// `None` = declaration drop ([`parse_border_shorthand`] と同じ契約)。
///
/// # Initial value fill (省略成分)
///
/// [`BackgroundShorthand`] doc の「Initial value fill」節参照。`visual_boxes`
/// の 0/1/2 occurrence による origin/clip 割り当ては spec §2.10 verbatim
/// ("If one `<visual-box>` value is present then it sets both
/// background-origin and background-clip to that value. If two values are
/// present, then the first sets background-origin and the second
/// background-clip.") — 0 個の場合は 2 longhand それぞれの spec initial
/// (origin: padding-box、clip: border-box) を使う点が 1 個の場合と異なる
/// (1 個の場合は両方その値になるため、0 個の場合だけ非対称)。
pub(crate) fn parse_background_shorthand(
    input: &mut Parser<'_, '_>,
) -> Option<BackgroundShorthand> {
    let mut image: Option<BackgroundImage> = None;
    let mut position_and_size: Option<(CssPosition, Option<BackgroundSize>)> = None;
    let mut repeat: Option<BackgroundRepeat> = None;
    let mut attachment: Option<BackgroundAttachment> = None;
    let mut visual_boxes: Vec<VisualBox> = Vec::new();
    let mut color: Option<CssColor> = None;

    loop {
        if image.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundImage, ParseError<'_, ()>> {
                parse_background_image(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            image = Some(v);
            continue;
        }
        if position_and_size.is_none()
            && let Ok(v) = input.try_parse(
                |i| -> Result<(CssPosition, Option<BackgroundSize>), ParseError<'_, ()>> {
                    parse_background_position_and_size(i).ok_or_else(|| i.new_custom_error(()))
                },
            )
        {
            position_and_size = Some(v);
            continue;
        }
        if repeat.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<BackgroundRepeat, ParseError<'_, ()>> {
                parse_background_repeat(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            repeat = Some(v);
            continue;
        }
        if attachment.is_none()
            && let Ok(v) =
                input.try_parse(|i| -> Result<BackgroundAttachment, ParseError<'_, ()>> {
                    parse_background_attachment(i).ok_or_else(|| i.new_custom_error(()))
                })
        {
            attachment = Some(v);
            continue;
        }
        if visual_boxes.len() < 2
            && let Ok(v) = input.try_parse(|i| -> Result<VisualBox, ParseError<'_, ()>> {
                parse_visual_box(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            visual_boxes.push(v);
            continue;
        }
        if color.is_none()
            && let Ok(v) = input.try_parse(|i| -> Result<CssColor, ParseError<'_, ()>> {
                parse_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(v);
            continue;
        }

        // どの unfilled slot にも match しなかった → 埋まっている slot に
        // 対する 2 回目の指定 (`visual_boxes` は 3 回目)、comma (複数 layer)、
        // または未知 token。break で loop 終了、caller の `expect_exhausted`
        // が leftover を drop する (`parse_border_shorthand` と同じ契約)。
        break;
    }

    if image.is_none()
        && position_and_size.is_none()
        && repeat.is_none()
        && attachment.is_none()
        && visual_boxes.is_empty()
        && color.is_none()
    {
        return None;
    }

    let (origin, clip) = match visual_boxes.as_slice() {
        [] => (VisualBox::PaddingBox, VisualBox::BorderBox),
        [one] => (*one, *one),
        [first, second] => (*first, *second),
        // cov:ignore: the loop above only pushes while
        // `visual_boxes.len() < 2`, so `visual_boxes` can never hold more
        // than 2 elements by the time this match runs — there is no input
        // that reaches this arm, and constructing one would require
        // bypassing the loop guard entirely.
        _ => unreachable!("visual_boxes never grows past 2 (loop guard above)"),
    };

    let (position, size) = match position_and_size {
        Some((position, size)) => (
            position,
            size.unwrap_or(BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            }),
        ),
        None => (
            CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
        ),
    };

    Some(BackgroundShorthand {
        color: color.unwrap_or(CssColor::TRANSPARENT),
        image: image.unwrap_or(BackgroundImage::None),
        repeat: repeat.unwrap_or(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        }),
        attachment: attachment.unwrap_or(BackgroundAttachment::Scroll),
        position,
        size,
        clip,
        origin,
    })
}

/// `<length>{2,4}` の box-shadow length run を parse する。
///
/// offset-x/offset-y/spread-radius はいずれも sign 制限なしのため、
/// [`parse_shadow_length_reject_nan`]/`_res` 経由で `!is_nan()` guard を
/// 通す (同関数 doc 参照)。blur-radius (3rd slot) は既存の
/// `length_payload(value) >= 0.0` チェックが NaN も incidental に
/// 弾くため、追加 guard は不要 (`NaN >= 0.0` は IEEE 754 で `false`)。
fn parse_box_shadow_lengths<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(Length, Length, Length, Length), ParseError<'i, ()>> {
    let offset_x = parse_shadow_length_reject_nan_res(input)?;
    let offset_y = parse_shadow_length_reject_nan_res(input)?;
    // Parse the optional third slot without rewinding a negative length into
    // the fourth (spread) slot.  The grammar's third length is blur-radius,
    // which is non-negative; only the fourth spread-radius may be negative.
    let blur_radius = match input.try_parse(parse_length_allow_negative_res) {
        Ok(value) if length_payload(value) >= 0.0 => value,
        Ok(_) => return Err(input.new_custom_error(())),
        Err(_) => Length::Px(0.0),
    };
    let spread_radius = input
        .try_parse(parse_shadow_length_reject_nan_res)
        .unwrap_or(Length::Px(0.0));
    Ok((offset_x, offset_y, blur_radius, spread_radius))
}

fn parse_box_shadow_item(input: &mut Parser<'_, '_>) -> Option<BoxShadowItem> {
    let mut lengths: Option<(Length, Length, Length, Length)> = None;
    let mut color: Option<TextShadowColor> = None;
    let mut inset = false;

    // The optional color and `inset` keyword may occur before or after the
    // required length run.
    loop {
        let mut progressed = false;
        if lengths.is_none()
            && let Ok(value) = input.try_parse(parse_box_shadow_lengths)
        {
            lengths = Some(value);
            progressed = true;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<TextShadowColor, ParseError<'_, ()>> {
                parse_text_shadow_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            progressed = true;
        }
        if !inset
            && input
                .try_parse(|i| i.expect_ident_matching("inset"))
                .is_ok()
        {
            inset = true;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }

    let (offset_x, offset_y, blur_radius, spread_radius) = lengths?;
    Some(BoxShadowItem {
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        color: color.unwrap_or(TextShadowColor::CurrentColor),
        inset,
    })
}

/// `box-shadow: none | <shadow>#` を parse する。
fn parse_box_shadow(input: &mut Parser<'_, '_>) -> Option<Vec<BoxShadowItem>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Some(Vec::new());
    }
    input
        .parse_comma_separated(|i| -> Result<BoxShadowItem, ParseError<'_, ()>> {
            parse_box_shadow_item(i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// `outline` shorthand の width/style/color components を any-order で parse する。
fn parse_outline(input: &mut Parser<'_, '_>) -> Option<Outline> {
    let mut width: Option<Length> = None;
    let mut style: Option<OutlineStyle> = None;
    let mut color: Option<OutlineColor> = None;

    loop {
        if width.is_some() && style.is_some() && color.is_some() {
            break;
        }
        if width.is_none()
            && let Ok(value) = input.try_parse(parse_border_width_side_res)
        {
            width = Some(value);
            continue;
        }
        if style.is_none()
            && let Ok(value) = input.try_parse(parse_outline_style_res)
        {
            style = Some(value);
            continue;
        }
        if color.is_none()
            && let Ok(value) = input.try_parse(|i| -> Result<OutlineColor, ParseError<'_, ()>> {
                parse_outline_color(i).ok_or_else(|| i.new_custom_error(()))
            })
        {
            color = Some(value);
            continue;
        }
        break;
    }

    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }
    Some(Outline {
        width: width.unwrap_or(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
        style: style.unwrap_or(OutlineStyle::None),
        color: color.unwrap_or(OutlineColor::Invert),
    })
}

/// `outline-color` の value parser。CSS UI 3 §4.4 の `invert | <color>` を受理し、
/// `currentcolor` は `<color>` の keyword として専用 variant に保持する。
fn parse_outline_color(input: &mut Parser<'_, '_>) -> Option<OutlineColor> {
    if input
        .try_parse(|i| i.expect_ident_matching("invert"))
        .is_ok()
    {
        return Some(OutlineColor::Invert);
    }
    if input
        .try_parse(|i| i.expect_ident_matching("currentcolor"))
        .is_ok()
    {
        return Some(OutlineColor::CurrentColor);
    }
    parse_color(input).map(OutlineColor::Resolved)
}

/// Parse one `outline-style` keyword. The outline shorthand and longhand share
/// this helper so `auto` cannot accidentally become valid for border styles.
fn parse_outline_style_side(input: &mut Parser<'_, '_>) -> Option<OutlineStyle> {
    let ident = input.expect_ident().ok()?.clone();
    match ident.to_ascii_lowercase().as_str() {
        "none" => Some(OutlineStyle::None),
        // Keep the existing `outline: hidden` rejection. The enum retains the
        // keyword for representation completeness, but this parser scope does
        // not accept it.
        "hidden" => None,
        "dotted" => Some(OutlineStyle::Dotted),
        "dashed" => Some(OutlineStyle::Dashed),
        "solid" => Some(OutlineStyle::Solid),
        "double" => Some(OutlineStyle::Double),
        "groove" => Some(OutlineStyle::Groove),
        "ridge" => Some(OutlineStyle::Ridge),
        "inset" => Some(OutlineStyle::Inset),
        "outset" => Some(OutlineStyle::Outset),
        "auto" => Some(OutlineStyle::Auto),
        _ => None,
    }
}

fn parse_outline_style_res<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<OutlineStyle, ParseError<'i, ()>> {
    parse_outline_style_side(input).ok_or_else(|| input.new_custom_error(()))
}
