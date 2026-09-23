//! Tests for the layout property parsers in `parse/layout.rs`.

use super::*;

// ── Display (CSS Display 3 §2) ─────────────

#[test]
fn display_parse_block() {
    assert_eq!(
        parse("block", "display"),
        Some(PropertyValue::Display(DisplayValue::Block))
    );
}

#[test]
fn display_parse_inline() {
    assert_eq!(
        parse("inline", "display"),
        Some(PropertyValue::Display(DisplayValue::Inline))
    );
}

#[test]
fn display_parse_inline_block() {
    // CSS Display 3 §2 <display-legacy>
    assert_eq!(
        parse("inline-block", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
}

#[test]
fn display_parse_flow_root() {
    assert_eq!(
        parse("flow-root", "display"),
        Some(PropertyValue::Display(DisplayValue::FlowRoot))
    );
    assert_eq!(
        parse("FLOW-ROOT", "display"),
        Some(PropertyValue::Display(DisplayValue::FlowRoot))
    );
}

#[test]
fn display_parse_none() {
    // CSS Display 3 §2 <display-box>
    assert_eq!(
        parse("none", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
}

#[test]
fn display_parse_flex() {
    // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
    // keyword, outer-defaulting rule makes it equivalent to `block flex`.
    assert_eq!(
        parse("flex", "display"),
        Some(PropertyValue::Display(DisplayValue::Flex))
    );
}

#[test]
fn display_parse_grid() {
    // CSS Display 3 §2.2 "Inner Display Layout Models" — `<display-inside>`
    // keyword, outer-defaulting rule makes it equivalent to `block grid`.
    assert_eq!(
        parse("grid", "display"),
        Some(PropertyValue::Display(DisplayValue::Grid))
    );
}

#[test]
fn display_parse_list_item() {
    // CSS Display 3 §2 <display-listitem>, outer-defaulting rule makes
    // it equivalent to `block flow list-item`. HTML Living Standard's
    // default UA stylesheet uses this for `li`
    // <https://html.spec.whatwg.org/multipage/rendering.html#lists>.
    assert_eq!(
        parse("list-item", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
}

#[test]
fn display_parse_contents() {
    // CSS Display 3 §2.5 <display-box> keyword — element generates no
    // box of its own, children/pseudo-elements still generate boxes.
    assert_eq!(
        parse("contents", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
}

// ── List styling (CSS Lists 3 §3) ─────────────

#[test]
fn list_style_values_have_css_initial_defaults() {
    assert_eq!(ListStyleType::default(), ListStyleType::Disc);
    assert_eq!(ListStylePosition::default(), ListStylePosition::Outside);
}

#[test]
fn list_style_type_parses_builtin_custom_and_string_values() {
    assert_eq!(
        parse_entire("disc", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::Disc))
    );
    assert_eq!(
        parse_entire("none", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::None))
    );
    assert_eq!(
        parse_entire("upper-roman", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::Named(
            "upper-roman".into()
        )))
    );
    assert_eq!(
        parse_entire("\"→\"", "list-style-type"),
        Some(PropertyValue::ListStyleType(ListStyleType::String(
            "→".into()
        )))
    );
}

#[test]
fn list_style_image_parses_none_and_url() {
    assert_eq!(
        parse_entire("none", "list-style-image"),
        Some(PropertyValue::ListStyleImage(BackgroundImage::None))
    );
    assert_eq!(
        parse_entire("url(marker.png)", "list-style-image"),
        Some(PropertyValue::ListStyleImage(BackgroundImage::Url(
            "marker.png".to_string()
        )))
    );
    assert_eq!(
        parse_entire("url(marker.png) none", "list-style-image"),
        None
    );
}

#[test]
fn list_style_type_rejects_reserved_or_trailing_values() {
    for value in [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ] {
        assert_eq!(parse_entire(value, "list-style-type"), None, "{value}");
    }
    assert_eq!(parse_entire("disc none", "list-style-type"), None);
    assert_eq!(parse_entire("url(marker.svg)", "list-style-type"), None);
}

#[test]
fn list_style_position_parses_case_insensitive_keywords_only() {
    assert_eq!(
        parse_entire("INSIDE", "list-style-position"),
        Some(PropertyValue::ListStylePosition(ListStylePosition::Inside))
    );
    assert_eq!(
        parse_entire("Outside", "list-style-position"),
        Some(PropertyValue::ListStylePosition(ListStylePosition::Outside))
    );
    assert_eq!(parse_entire("middle", "list-style-position"), None);
    assert_eq!(parse_entire("inside outside", "list-style-position"), None);
}

#[test]
fn display_rejects_unknown_ident() {
    // inline-flex / inline-grid / flow-root are accepted as their
    // corresponding formatting contexts.
    assert_eq!(
        parse("inline-flex", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineFlex))
    );
    assert_eq!(
        parse("inline-grid", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineGrid))
    );
    assert_eq!(
        parse_entire("flow-root extra", "display"),
        None,
        // cov:ignore: panic-message literal only executes on assertion failure.
        "display accepts one standalone keyword in this slice"
    );
}

#[test]
fn display_rejects_non_ident() {
    assert_eq!(parse("16px", "display"), None);
    assert_eq!(parse("100", "display"), None);
}

#[test]
fn display_is_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
    assert_eq!(
        parse("BLOCK", "display"),
        Some(PropertyValue::Display(DisplayValue::Block))
    );
    assert_eq!(
        parse("Inline", "display"),
        Some(PropertyValue::Display(DisplayValue::Inline))
    );
    assert_eq!(
        parse("INLINE-BLOCK", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
    assert_eq!(
        parse("Inline-Block", "display"),
        Some(PropertyValue::Display(DisplayValue::InlineBlock))
    );
    assert_eq!(
        parse("NONE", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
    assert_eq!(
        parse("None", "display"),
        Some(PropertyValue::Display(DisplayValue::None))
    );
    assert_eq!(
        parse("FLEX", "display"),
        Some(PropertyValue::Display(DisplayValue::Flex))
    );
    assert_eq!(
        parse("Grid", "display"),
        Some(PropertyValue::Display(DisplayValue::Grid))
    );
    assert_eq!(
        parse("LIST-ITEM", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
    assert_eq!(
        parse("List-Item", "display"),
        Some(PropertyValue::Display(DisplayValue::ListItem))
    );
    assert_eq!(
        parse("CONTENTS", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
    assert_eq!(
        parse("Contents", "display"),
        Some(PropertyValue::Display(DisplayValue::Contents))
    );
}

// ── float (CSS2 §9.5.1) ──
//
// Value grammar (§9.5.1 spec verbatim): `left | right | none | inherit`.
// Initial: none / Inherited: no / Computed value: as specified.

#[test]
fn float_parse_all_keywords() {
    assert_eq!(
        parse("none", "float"),
        Some(PropertyValue::Float(FloatValue::None))
    );
    assert_eq!(
        parse("left", "float"),
        Some(PropertyValue::Float(FloatValue::Left))
    );
    assert_eq!(
        parse("right", "float"),
        Some(PropertyValue::Float(FloatValue::Right))
    );
    assert_eq!(
        parse("footnote", "float"),
        Some(PropertyValue::Float(FloatValue::Footnote))
    );
}

#[test]
fn float_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "float"),
        Some(PropertyValue::Float(FloatValue::None))
    );
    assert_eq!(
        parse("Left", "float"),
        Some(PropertyValue::Float(FloatValue::Left))
    );
    assert_eq!(
        parse("RIGHT", "float"),
        Some(PropertyValue::Float(FloatValue::Right))
    );
}

#[test]
fn float_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "float"), None);
    // `clear`'s `both` keyword is not valid on `float`.
    assert_eq!(parse("both", "float"), None);
}

#[test]
fn float_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "float"), None);
    }
}

#[test]
fn float_rejects_non_ident() {
    assert_eq!(parse("16px", "float"), None);
    assert_eq!(parse(r#""left""#, "float"), None);
}

#[test]
fn float_key_maps_to_float_property_key() {
    let v = PropertyValue::Float(FloatValue::None);
    assert_eq!(v.key(), PropertyKey::Float);
    let v = PropertyValue::Float(FloatValue::Left);
    assert_eq!(v.key(), PropertyKey::Float);
    let v = PropertyValue::Float(FloatValue::Right);
    assert_eq!(v.key(), PropertyKey::Float);
}

// ── clear (CSS2 §9.5.2) ──
//
// Value grammar (§9.5.2 spec verbatim): `none | left | right | both |
// inherit`. Initial: none / Inherited: no / Computed value: as
// specified.

#[test]
fn clear_parse_all_keywords() {
    assert_eq!(
        parse("none", "clear"),
        Some(PropertyValue::Clear(ClearValue::None))
    );
    assert_eq!(
        parse("left", "clear"),
        Some(PropertyValue::Clear(ClearValue::Left))
    );
    assert_eq!(
        parse("right", "clear"),
        Some(PropertyValue::Clear(ClearValue::Right))
    );
    assert_eq!(
        parse("both", "clear"),
        Some(PropertyValue::Clear(ClearValue::Both))
    );
}

#[test]
fn clear_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "clear"),
        Some(PropertyValue::Clear(ClearValue::None))
    );
    assert_eq!(
        parse("Both", "clear"),
        Some(PropertyValue::Clear(ClearValue::Both))
    );
}

#[test]
fn clear_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "clear"), None);
}

#[test]
fn clear_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "clear"), None);
    }
}

#[test]
fn clear_rejects_non_ident() {
    assert_eq!(parse("16px", "clear"), None);
    assert_eq!(parse(r#""left""#, "clear"), None);
}

#[test]
fn clear_key_maps_to_clear_property_key() {
    let v = PropertyValue::Clear(ClearValue::None);
    assert_eq!(v.key(), PropertyKey::Clear);
    let v = PropertyValue::Clear(ClearValue::Both);
    assert_eq!(v.key(), PropertyKey::Clear);
}

// ── resolve_display_for_float (CSS2 §9.7) ──

#[test]
fn resolve_display_for_float_is_noop_when_float_is_none() {
    for display in [
        DisplayValue::Block,
        DisplayValue::Inline,
        DisplayValue::InlineBlock,
        DisplayValue::None,
        DisplayValue::Flex,
        DisplayValue::Grid,
        DisplayValue::ListItem,
        DisplayValue::FlowRoot,
        DisplayValue::Contents,
    ] {
        assert_eq!(
            resolve_display_for_float(display, FloatValue::None),
            display
        );
    }
}

#[test]
fn resolve_display_for_float_forces_inline_and_inline_block_to_block() {
    // §9.7 table: `inline` / `inline-block` → `block`.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Inline, float),
            DisplayValue::Block
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::InlineBlock, float),
            DisplayValue::Block
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_block_flex_grid_list_item_unchanged() {
    // §9.7 table's "others" row — not in the forced-to-block list.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Block, float),
            DisplayValue::Block
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::Flex, float),
            DisplayValue::Flex
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::Grid, float),
            DisplayValue::Grid
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::ListItem, float),
            DisplayValue::ListItem
        );
        assert_eq!(
            resolve_display_for_float(DisplayValue::FlowRoot, float),
            DisplayValue::FlowRoot
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_none_as_none() {
    // §9.7 leading clause: "If 'display' has the value 'none', then
    // 'position' and 'float' do not apply" — this precedes the table,
    // so `none` must not be affected even when `float` is not `none`.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::None, float),
            DisplayValue::None
        );
    }
}

#[test]
fn resolve_display_for_float_leaves_contents_unchanged() {
    // CSS Display 3 §2.7 verbatim: blockification "has no effect on
    // display types that generate no box at all, such as display:
    // none or display: contents" — so floating a `contents` element
    // must not force it to `block` either.
    for float in [FloatValue::Left, FloatValue::Right] {
        assert_eq!(
            resolve_display_for_float(DisplayValue::Contents, float),
            DisplayValue::Contents
        );
    }
}

// ── break-before / break-after (CSS Fragmentation Module Level 3
// §3.1) + page-break-before / page-break-after legacy shorthand (§3.4) ──
//
// Value grammar (this crate's scope, `BreakBetween` doc's "Scope
// carving" section): `auto | avoid | avoid-page | page`. Initial:
// `auto` / Inherited: no / Computed value: specified keyword.

#[test]
fn break_before_after_parse_all_implemented_keywords() {
    assert_eq!(
        parse("auto", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Auto))
    );
    assert_eq!(
        parse("auto", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Auto))
    );
    assert_eq!(
        parse("avoid", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
    assert_eq!(
        parse("avoid", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("page", "break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Page))
    );
}

#[test]
fn break_before_is_case_insensitive() {
    assert_eq!(
        parse("AUTO", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Auto))
    );
    assert_eq!(
        parse("Avoid-Page", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::AvoidPage))
    );
    assert_eq!(
        parse("PAGE", "break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
}

#[test]
fn break_before_after_rejects_out_of_scope_column_and_region_values() {
    // (b) not supported — this crate has no multi-column or CSS
    // Regions fragmentation context (`BreakBetween` doc's "Scope
    // carving" section), not (a) spec-invalid.
    for kw in ["avoid-column", "column", "avoid-region", "region"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_out_of_scope_page_spread_values() {
    // (b) not supported — no page-spread concept in this crate
    // (`BreakBetween` doc's "Scope carving" section).
    for kw in ["left", "right", "recto", "verso"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_always_and_all() {
    // `always`/`all` are not part of the current break-before/
    // break-after grammar at all (`BreakBetween` doc's "Scope carving"
    // section — Level 3's change log only names `always`, not `all`;
    // both live in Level 4 instead, which this crate does not target).
    // `always` is valid only as the `page-break-before`/
    // `page-break-after` legacy shorthand's own keyword, never
    // directly on `break-before`/`break-after`; `all` has no path in
    // at all.
    for kw in ["always", "all"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "break-before"), None);
    assert_eq!(parse("bogus", "break-after"), None);
}

#[test]
fn break_before_after_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "break-before"), None);
        assert_eq!(parse(kw, "break-after"), None);
    }
}

#[test]
fn break_before_after_rejects_non_ident() {
    assert_eq!(parse("16px", "break-before"), None);
    assert_eq!(parse(r#""auto""#, "break-after"), None);
}

#[test]
fn break_before_after_key_maps_to_distinct_property_keys() {
    // `break-before` / `break-after` are 2 independent cascade winners
    // (unlike the `word-wrap`/`overflow-wrap` name alias, which shares
    // one `PropertyKey` — `BreakBetween` doc's "legacy shorthand"
    // section explains why this pair does too, just each with its
    // *own* longhand).
    let v = PropertyValue::BreakBefore(BreakBetween::Page);
    assert_eq!(v.key(), PropertyKey::BreakBefore);
    let v = PropertyValue::BreakAfter(BreakBetween::Page);
    assert_eq!(v.key(), PropertyKey::BreakAfter);
}

// ── page-break-before / page-break-after legacy shorthand value remap
// (CSS Fragmentation Module Level 3 §3.4) ──

#[test]
fn page_break_before_after_legacy_shorthand_identity_values() {
    for prop in ["page-break-before", "page-break-after"] {
        let wrap = |v| {
            if prop == "page-break-before" {
                PropertyValue::BreakBefore(v)
            } else {
                PropertyValue::BreakAfter(v)
            }
        };
        assert_eq!(parse("auto", prop), Some(wrap(BreakBetween::Auto)));
        assert_eq!(parse("avoid", prop), Some(wrap(BreakBetween::Avoid)));
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_remaps_always_to_page() {
    // CSS Fragmentation Module Level 3 §3.4 mapping table verbatim:
    // `always` (page-break-*) -> `page` (break-*). Non-identity remap —
    // check the exact equality with the longhand spelling, mirroring
    // `word_wrap_legacy_alias_parses_identically_to_overflow_wrap`'s
    // shape (there the two spellings are identical; here they are not,
    // which is exactly what this test must catch).
    assert_eq!(
        parse("always", "page-break-before"),
        Some(PropertyValue::BreakBefore(BreakBetween::Page))
    );
    assert_eq!(
        parse("always", "page-break-before"),
        parse("page", "break-before")
    );
    assert_eq!(
        parse("always", "page-break-after"),
        Some(PropertyValue::BreakAfter(BreakBetween::Page))
    );
    assert_eq!(
        parse("always", "page-break-after"),
        parse("page", "break-after")
    );
}

#[test]
fn page_break_before_after_legacy_shorthand_rejects_new_property_only_values() {
    // `avoid-page` / `page` are valid on `break-before`/`break-after`
    // directly, but CSS2.1's own `page-break-before`/`page-break-after`
    // propdef grammar (`auto | always | avoid | left | right`) does not
    // have them — the legacy shorthand's grammar is CSS2.1's, not the
    // new property's (`BreakBetween` doc's "legacy shorthand" section).
    for kw in ["avoid-page", "page"] {
        assert_eq!(parse(kw, "page-break-before"), None);
        assert_eq!(parse(kw, "page-break-after"), None);
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_rejects_left_and_right() {
    // CSS2.1's own grammar has `left`/`right`, but they remap to
    // `break-before`/`break-after` values this crate does not
    // implement (`BreakBetween` doc's "Scope carving" section) — the
    // scope carve applies transitively through the legacy shorthand.
    for kw in ["left", "right"] {
        assert_eq!(parse(kw, "page-break-before"), None);
        assert_eq!(parse(kw, "page-break-after"), None);
    }
}

#[test]
fn page_break_before_after_legacy_shorthand_key_maps_to_same_key_as_longhand() {
    // One cascade winner per property, whether declared via the new
    // name or the legacy shorthand name (`BreakBetween` doc's "legacy
    // shorthand" section — no `expand_shorthand_into` arm needed since
    // this is a 1:1, not a fan-out, shorthand).
    let v = parse("always", "page-break-before").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakBefore);
    let v = parse("always", "page-break-after").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakAfter);
}

// ── break-inside (CSS Fragmentation Module Level 3 §3.2) +
// page-break-inside legacy shorthand (§3.4) ──
//
// Value grammar (this crate's scope, `BreakInside` doc's "Scope
// carving" section): `auto | avoid | avoid-page` — a smaller, disjoint
// set from `break-before`/`break-after`'s `BreakBetween` (no `page`).
// Initial: `auto` / Inherited: no / Computed value: specified keyword.

#[test]
fn break_inside_parse_all_implemented_keywords() {
    assert_eq!(
        parse("auto", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("avoid", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Avoid))
    );
    assert_eq!(
        parse("avoid-page", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
    );
}

#[test]
fn break_inside_is_case_insensitive() {
    assert_eq!(
        parse("AUTO", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("Avoid-Page", "break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::AvoidPage))
    );
}

#[test]
fn break_inside_rejects_forced_break_values() {
    // `break-inside` has no forced-break values at all — `page` /
    // `column` / `region` are valid on `break-before`/`break-after`
    // (or would be, absent this crate's scope carve) but have no
    // `break-inside` counterpart in the spec grammar at all, not even
    // an excluded one (`BreakInside` doc: "breaking within has no
    // start/end edge to force a break relative to").
    for kw in ["page", "column", "region"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_out_of_scope_avoid_values() {
    // Unlike `page`/`column`/`region` above, `avoid-column` and
    // `avoid-region` *are* in `break-inside`'s own propdef grammar
    // (`auto | avoid | avoid-page | avoid-column | avoid-region`,
    // `BreakInside` doc) — they are rejected here purely by this
    // crate's scope carve (no multi-column / CSS Regions fragmentation
    // context), not because the spec lacks them.
    for kw in ["avoid-column", "avoid-region"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "break-inside"), None);
}

#[test]
fn break_inside_rejects_css_wide_keyword() {
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "break-inside"), None);
    }
}

#[test]
fn break_inside_rejects_non_ident() {
    assert_eq!(parse("16px", "break-inside"), None);
    assert_eq!(parse(r#""auto""#, "break-inside"), None);
}

#[test]
fn break_inside_key_maps_to_break_inside_property_key() {
    let v = PropertyValue::BreakInside(BreakInside::AvoidPage);
    assert_eq!(v.key(), PropertyKey::BreakInside);
}

#[test]
fn page_break_inside_legacy_shorthand_identity_values() {
    assert_eq!(
        parse("auto", "page-break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Auto))
    );
    assert_eq!(
        parse("avoid", "page-break-inside"),
        Some(PropertyValue::BreakInside(BreakInside::Avoid))
    );
    assert_eq!(
        parse("auto", "page-break-inside"),
        parse("auto", "break-inside")
    );
    assert_eq!(
        parse("avoid", "page-break-inside"),
        parse("avoid", "break-inside")
    );
}

#[test]
fn page_break_inside_legacy_shorthand_rejects_new_property_only_value() {
    // `avoid-page` is valid on `break-inside` directly, but CSS2.1's
    // own `page-break-inside` propdef grammar is just `auto | avoid` —
    // the legacy shorthand's grammar is CSS2.1's, not the new
    // property's fuller one (`BreakInside` doc's "legacy shorthand"
    // section).
    assert_eq!(parse("avoid-page", "page-break-inside"), None);
}

#[test]
fn page_break_inside_legacy_shorthand_key_maps_to_same_key_as_longhand() {
    let v = parse("avoid", "page-break-inside").unwrap();
    assert_eq!(v.key(), PropertyKey::BreakInside);
}

// ── flex-direction (CSS Flexible Box Layout Module Level 1 §5.1) ────

#[test]
fn flex_direction_parse_all_keywords() {
    assert_eq!(
        parse("row", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::Row))
    );
    assert_eq!(
        parse("row-reverse", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::RowReverse))
    );
    assert_eq!(
        parse("column", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::Column))
    );
    assert_eq!(
        parse("column-reverse", "flex-direction"),
        Some(PropertyValue::FlexDirection(
            FlexDirectionValue::ColumnReverse
        ))
    );
}

#[test]
fn flex_direction_case_insensitive() {
    assert_eq!(
        parse("ROW-REVERSE", "flex-direction"),
        Some(PropertyValue::FlexDirection(FlexDirectionValue::RowReverse))
    );
}

#[test]
fn flex_direction_rejects_unknown_ident() {
    assert_eq!(parse("diagonal", "flex-direction"), None);
}

// ── flex-wrap (CSS Flexible Box Layout Module Level 1 §5.2) ─────────

#[test]
fn flex_wrap_parse_all_keywords() {
    assert_eq!(
        parse("nowrap", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::NoWrap))
    );
    assert_eq!(
        parse("wrap", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::Wrap))
    );
    assert_eq!(
        parse("wrap-reverse", "flex-wrap"),
        Some(PropertyValue::FlexWrap(FlexWrapValue::WrapReverse))
    );
}

#[test]
fn flex_wrap_rejects_unknown_ident() {
    assert_eq!(parse("nowrap-ish", "flex-wrap"), None);
}

// ── flex-grow / flex-shrink (CSS Flexible Box Layout Module Level 1
//    §7.2.1 / §7.2.2) ──────────────────────────────────────────────

#[test]
fn flex_grow_parse_number() {
    assert_eq!(parse("2", "flex-grow"), Some(PropertyValue::FlexGrow(2.0)));
    assert_eq!(parse("0", "flex-grow"), Some(PropertyValue::FlexGrow(0.0)));
    assert_eq!(
        parse("1.5", "flex-grow"),
        Some(PropertyValue::FlexGrow(1.5))
    );
}

#[test]
fn flex_grow_rejects_negative() {
    // spec `<number [0,∞]>` — negative は grammar 違反。
    assert_eq!(parse("-1", "flex-grow"), None);
}

#[test]
fn flex_grow_rejects_non_finite_literal() {
    // f64 → f32 変換で `+Inf` になる巨大 literal
    // (`parse_nonneg_finite_number` doc の hazard 節参照)。
    assert_eq!(parse("1e40", "flex-grow"), None);
}

#[test]
fn flex_shrink_parse_number() {
    assert_eq!(
        parse("3", "flex-shrink"),
        Some(PropertyValue::FlexShrink(3.0))
    );
}

#[test]
fn flex_shrink_rejects_negative() {
    assert_eq!(parse("-2", "flex-shrink"), None);
}

// ── flex-basis (CSS Flexible Box Layout Module Level 1 §7.2.3) ──────

#[test]
fn flex_basis_parse_auto() {
    assert_eq!(
        parse("auto", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Auto))
    );
}

#[test]
fn flex_basis_parse_content() {
    assert_eq!(
        parse("content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Content))
    );
}

#[test]
fn flex_basis_parse_min_max_fit_content_keywords() {
    // CSS Sizing 3 intrinsic keywords (WPT `flex-basis-valid.html`).
    // Bare `fit-content` only — the `fit-content(<length-percentage>)`
    // function form stays out of scope.
    assert_eq!(
        parse("min-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::MinContent))
    );
    assert_eq!(
        parse("max-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::MaxContent))
    );
    assert_eq!(
        parse("fit-content", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::FitContent))
    );
}

#[test]
fn flex_basis_parse_length_and_percentage() {
    assert_eq!(
        parse("200px", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Length(
            Length::Px(200.0)
        )))
    );
    assert_eq!(
        parse("50%", "flex-basis"),
        Some(PropertyValue::FlexBasis(FlexBasisValue::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn flex_basis_rejects_negative_length() {
    // `<'width'>` reuse — CSS Sizing 3 §3.1.1 の `[0,∞]` constraint。
    assert_eq!(parse("-10px", "flex-basis"), None);
}

// ── flex shorthand (CSS Flexible Box Layout Module Level 1 §7.1) ────

#[test]
fn flex_shorthand_none_expands_to_0_0_auto() {
    assert_eq!(
        parse("none", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 0.0,
            shrink: 0.0,
            basis: FlexBasisValue::Auto,
        }))
    );
}

#[test]
fn flex_shorthand_auto_is_1_1_auto() {
    // §7.1.1 informative summary: `flex: auto` == `flex: 1 1 auto`。
    assert_eq!(
        parse("auto", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Auto,
        }))
    );
}

#[test]
fn flex_shorthand_bare_number_defaults_shrink_1_basis_0() {
    // §7.1.1 informative summary: `flex: <number [1,∞]>` ==
    // `flex: <number> 1 0` — omitted-component default (grow=1/shrink=1
    // であって longhand 自身の initial ではない、`FlexShorthand` doc の
    // "Omitted-component defaults" 節)。
    assert_eq!(
        parse("2", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_unitless_zero_is_a_flex_factor_not_yet_preceded_by_two() {
    // spec §7.1 verbatim: "A unitless zero that is not already preceded
    // by two flex factors must be interpreted as a flex factor."
    assert_eq!(
        parse("0", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 0.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_unitless_zero_after_two_factors_is_basis() {
    // 同じ spec 文の逆方向 — 2 つの flex factor の**後**の unitless zero は
    // flex-basis として解釈される。
    assert_eq!(
        parse("2 3 0", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Px(0.0)),
        }))
    );
}

#[test]
fn flex_shorthand_basis_only_defaults_grow_1_shrink_1() {
    assert_eq!(
        parse("30px", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(30.0)),
        }))
    );
}

#[test]
fn flex_shorthand_basis_before_grow_shrink() {
    // `||` combinator — order-independent between the 2 groups.
    assert_eq!(
        parse("300px 2", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 1.0,
            basis: FlexBasisValue::Length(Length::Px(300.0)),
        }))
    );
}

#[test]
fn flex_shorthand_grow_shrink_basis_full_form() {
    assert_eq!(
        parse("2 3 10%", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Percent(10.0)),
        }))
    );
}

#[test]
fn flex_shorthand_content_basis() {
    assert_eq!(
        parse("1 1 content", "flex"),
        Some(PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Content,
        }))
    );
}

#[test]
fn flex_shorthand_empty_is_none() {
    assert_eq!(parse("", "flex"), None);
}

// ── flex-flow shorthand (CSS Flexible Box Layout Module Level 1 §5.3)

#[test]
fn flex_flow_direction_only() {
    assert_eq!(
        parse("column", "flex-flow"),
        Some(PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::Column,
            wrap: FlexWrapValue::NoWrap,
        }))
    );
}

#[test]
fn flex_flow_wrap_only() {
    assert_eq!(
        parse("wrap", "flex-flow"),
        Some(PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::Row,
            wrap: FlexWrapValue::Wrap,
        }))
    );
}

#[test]
fn flex_flow_both_components_either_order() {
    let both = || {
        PropertyValue::FlexFlow(FlexFlow {
            direction: FlexDirectionValue::RowReverse,
            wrap: FlexWrapValue::WrapReverse,
        })
    };
    assert_eq!(parse("row-reverse wrap-reverse", "flex-flow"), Some(both()));
    assert_eq!(parse("wrap-reverse row-reverse", "flex-flow"), Some(both()));
}

#[test]
fn flex_flow_rejects_empty_and_unknown() {
    assert_eq!(parse("", "flex-flow"), None);
    assert_eq!(parse("diagonal", "flex-flow"), None);
}

#[test]
fn flex_flow_rejects_duplicate_components() {
    // `parse_value` 契約では leftover token を consume せず caller
    // (`DeclParser` の `expect_exhausted`) が declaration ごと drop する
    // (`parse_flex_shorthand` と同じ contract) — ここでは
    // `parse_entire` で declaration-level の exhaustiveness を再現する。
    assert_eq!(parse_entire("row row", "flex-flow"), None);
    assert_eq!(parse_entire("wrap wrap", "flex-flow"), None);
    assert_eq!(parse_entire("row wrap nowrap", "flex-flow"), None);
}

#[test]
fn flex_flow_key_maps_to_flex_flow_property_key() {
    let v = PropertyValue::FlexFlow(FlexFlow {
        direction: FlexDirectionValue::Row,
        wrap: FlexWrapValue::NoWrap,
    });
    assert_eq!(v.key(), PropertyKey::FlexFlow);
}

// ── order (CSS Flexible Box Layout Module Level 1 §4.2) ────────────

#[test]
fn order_parses_integers() {
    assert_eq!(parse("0", "order"), Some(PropertyValue::Order(0)));
    assert_eq!(parse("3", "order"), Some(PropertyValue::Order(3)));
    assert_eq!(parse("-1", "order"), Some(PropertyValue::Order(-1)));
}

#[test]
fn order_rejects_non_integers() {
    assert_eq!(parse("auto", "order"), None);
    assert_eq!(parse("1.5", "order"), None);
    assert_eq!(parse("row", "order"), None);
}

#[test]
fn order_key_maps_to_order_property_key() {
    let v = PropertyValue::Order(2);
    assert_eq!(v.key(), PropertyKey::Order);
}

// ── justify-content / align-content (CSS Box Alignment Module Level 3
//    §5.1) — shared `ContentAlignmentValue` ─────────────────────────

#[test]
fn justify_content_parse_content_distribution_and_position() {
    assert_eq!(
        parse("space-between", "justify-content"),
        Some(PropertyValue::JustifyContent(
            ContentAlignmentValue::SpaceBetween
        ))
    );
    assert_eq!(
        parse("center", "justify-content"),
        Some(PropertyValue::JustifyContent(ContentAlignmentValue::Center))
    );
    assert_eq!(
        parse("flex-end", "justify-content"),
        Some(PropertyValue::JustifyContent(
            ContentAlignmentValue::FlexEnd
        ))
    );
    assert_eq!(
        parse("normal", "justify-content"),
        Some(PropertyValue::JustifyContent(ContentAlignmentValue::Normal))
    );
}

#[test]
fn align_content_parse_same_grammar_as_justify_content() {
    assert_eq!(
        parse("stretch", "align-content"),
        Some(PropertyValue::AlignContent(ContentAlignmentValue::Stretch))
    );
    assert_eq!(
        parse("space-evenly", "align-content"),
        Some(PropertyValue::AlignContent(
            ContentAlignmentValue::SpaceEvenly
        ))
    );
}

#[test]
fn content_alignment_rejects_left_right_and_baseline() {
    // scope carving: `left`/`right` (justify-content-specific extension)
    // と `<baseline-position>` は taffy に対応 variant が無いため未実装
    // (`ContentAlignmentValue` doc 参照)。
    assert_eq!(parse("left", "justify-content"), None);
    assert_eq!(parse("right", "justify-content"), None);
    assert_eq!(parse("baseline", "align-content"), None);
}

#[test]
fn content_alignment_rejects_overflow_position_prefix() {
    // scope carving: `safe`/`unsafe` prefix は未実装。
    assert_eq!(parse("safe center", "justify-content"), None);
}

// ── align-items (CSS Box Alignment Module Level 3 §7.2) ─────────────

#[test]
fn align_items_parse_self_position_and_baseline() {
    assert_eq!(
        parse("stretch", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Stretch))
    );
    assert_eq!(
        parse("baseline", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Baseline))
    );
    assert_eq!(
        parse("flex-start", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::FlexStart))
    );
}

#[test]
fn align_items_rejects_auto() {
    // `auto` は align-self 専用 keyword — align-items の grammar には無い。
    assert_eq!(parse("auto", "align-items"), None);
}

#[test]
fn align_items_rejects_self_start_self_end() {
    // scope carving: writing-mode 相対 keyword は未実装
    // (`SelfAlignmentValue` doc 参照)。
    assert_eq!(
        parse("self-start", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::Start))
    );
    assert_eq!(
        parse("self-end", "align-items"),
        Some(PropertyValue::AlignItems(SelfAlignmentValue::End))
    );
}

// ── align-self (CSS Box Alignment Module Level 3 §6.2) ──────────────

#[test]
fn align_self_parse_auto() {
    assert_eq!(
        parse("auto", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Auto))
    );
}

#[test]
fn align_self_parse_explicit_reuses_self_alignment_grammar() {
    assert_eq!(
        parse("center", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Value(
            SelfAlignmentValue::Center
        )))
    );
    assert_eq!(
        parse("baseline", "align-self"),
        Some(PropertyValue::AlignSelf(AlignSelfValue::Value(
            SelfAlignmentValue::Baseline
        )))
    );
}

// ── row-gap / column-gap (CSS Box Alignment Module Level 3 §8.1) ────

#[test]
fn row_gap_parse_normal() {
    assert_eq!(
        parse("normal", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Normal))
    );
}

#[test]
fn row_gap_parse_length_and_percentage() {
    assert_eq!(
        parse("10px", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Length(Length::Px(
            10.0
        ))))
    );
    assert_eq!(
        parse("5%", "row-gap"),
        Some(PropertyValue::RowGap(LengthOrNormal::Length(
            Length::Percent(5.0)
        )))
    );
}

#[test]
fn row_gap_rejects_negative() {
    assert_eq!(parse("-1px", "row-gap"), None);
}

#[test]
fn column_gap_parse_same_grammar_as_row_gap() {
    assert_eq!(
        parse("2em", "column-gap"),
        Some(PropertyValue::ColumnGap(LengthOrNormal::Length(
            Length::Em(2.0)
        )))
    );
}

// ── gap shorthand (CSS Box Alignment Module Level 3 §8.2) ───────────

#[test]
fn gap_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("10px", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(10.0)),
        }))
    );
}

#[test]
fn gap_shorthand_two_values() {
    assert_eq!(
        parse("10px 20px", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(20.0)),
        }))
    );
}

#[test]
fn gap_shorthand_normal() {
    assert_eq!(
        parse("normal", "gap"),
        Some(PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Normal,
            column: LengthOrNormal::Normal,
        }))
    );
}

// ── place-content shorthand (CSS Box Alignment Module Level 3 §5.2) ─

#[test]
fn place_content_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("center", "place-content"),
        Some(PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Center,
            justify: ContentAlignmentValue::Center,
        }))
    );
}

#[test]
fn place_content_shorthand_two_values() {
    assert_eq!(
        parse("center space-between", "place-content"),
        Some(PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Center,
            justify: ContentAlignmentValue::SpaceBetween,
        }))
    );
}

// ── key() discriminant integrity (mirrors `height_key_maps_to_height_property_key`) ─

#[test]
fn flex_group_keys_map_correctly() {
    assert_eq!(
        PropertyValue::FlexDirection(FlexDirectionValue::Row).key(),
        PropertyKey::FlexDirection
    );
    assert_eq!(
        PropertyValue::FlexWrap(FlexWrapValue::NoWrap).key(),
        PropertyKey::FlexWrap
    );
    assert_eq!(PropertyValue::FlexGrow(1.0).key(), PropertyKey::FlexGrow);
    assert_eq!(
        PropertyValue::FlexShrink(1.0).key(),
        PropertyKey::FlexShrink
    );
    assert_eq!(
        PropertyValue::FlexBasis(FlexBasisValue::Auto).key(),
        PropertyKey::FlexBasis
    );
    assert_eq!(
        PropertyValue::Flex(FlexShorthand {
            grow: 1.0,
            shrink: 1.0,
            basis: FlexBasisValue::Auto,
        })
        .key(),
        PropertyKey::Flex
    );
    assert_eq!(
        PropertyValue::JustifyContent(ContentAlignmentValue::Normal).key(),
        PropertyKey::JustifyContent
    );
    assert_eq!(
        PropertyValue::AlignContent(ContentAlignmentValue::Normal).key(),
        PropertyKey::AlignContent
    );
    assert_eq!(
        PropertyValue::AlignItems(SelfAlignmentValue::Normal).key(),
        PropertyKey::AlignItems
    );
    assert_eq!(
        PropertyValue::AlignSelf(AlignSelfValue::Auto).key(),
        PropertyKey::AlignSelf
    );
    assert_eq!(
        PropertyValue::RowGap(LengthOrNormal::Normal).key(),
        PropertyKey::RowGap
    );
    assert_eq!(
        PropertyValue::ColumnGap(LengthOrNormal::Normal).key(),
        PropertyKey::ColumnGap
    );
    assert_eq!(
        PropertyValue::Gap(GapShorthand {
            row: LengthOrNormal::Normal,
            column: LengthOrNormal::Normal,
        })
        .key(),
        PropertyKey::Gap
    );
    assert_eq!(
        PropertyValue::PlaceContent(PlaceContentShorthand {
            align: ContentAlignmentValue::Normal,
            justify: ContentAlignmentValue::Normal,
        })
        .key(),
        PropertyKey::PlaceContent
    );
}

// ── grid-template-columns / grid-template-rows (CSS Grid Layout Module
//    Level 1 §7.2) ──────────────────────────────────────────────────

#[test]
fn grid_template_columns_none() {
    assert_eq!(
        parse("none", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::None))
    );
}

#[test]
fn grid_template_columns_single_track() {
    assert_eq!(
        parse("100px", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Length(Length::Px(100.0))
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_multiple_tracks_and_keywords() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "100px auto 1fr min-content max-content",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(list.components.len(), 5);
    assert_eq!(
        list.components[0],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Length(
            Length::Px(100.0)
        )))
    );
    assert_eq!(
        list.components[1],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Auto))
    );
    assert_eq!(
        list.components[2],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0)))
    );
    assert_eq!(
        list.components[3],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::MinContent))
    );
    assert_eq!(
        list.components[4],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::MaxContent))
    );
    // 6 line-name slots (5 components + 1 trailing), all empty.
    assert_eq!(list.line_names.len(), 6);
    assert!(list.line_names.iter().all(Vec::is_empty));
}

#[test]
fn grid_template_columns_minmax() {
    assert_eq!(
        parse("minmax(0, 1fr)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::MinMax(
                    GridInflexibleBreadth::Length(Length::Px(0.0)),
                    GridTrackBreadth::Flex(1.0),
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_minmax_rejects_flex_in_min_position() {
    // `<inflexible-breadth>` (minmax's first argument) excludes `<flex>`
    // — CSS Grid Layout Module Level 1 §7.2.1.
    assert_eq!(parse("minmax(1fr, 100px)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_minmax_accepts_all_inflexible_breadth_keywords_as_min() {
    // `<inflexible-breadth>` (`minmax()`'s first argument) accepts
    // `auto` / `min-content` / `max-content` in addition to
    // `<length-percentage>` (already covered by `grid_template_columns_minmax`
    // above) — CSS Grid Layout Module Level 1 §7.2.1.
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "minmax(auto, 100px) minmax(min-content, 1fr) minmax(max-content, 1fr)",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::Auto,
                GridTrackBreadth::Length(Length::Px(100.0)),
            )),
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::MinContent,
                GridTrackBreadth::Flex(1.0),
            )),
            GridTrackListComponent::Size(GridTrackSize::MinMax(
                GridInflexibleBreadth::MaxContent,
                GridTrackBreadth::Flex(1.0),
            )),
        ]
    );
}

#[test]
fn grid_template_columns_rejects_negative_fr() {
    // `<flex [0,∞]>` (CSS Grid Layout Module Level 1 §7.2.4) — a
    // negative `fr` value fails `parse_grid_flex_res`'s non-negative
    // check, and (unlike a valid `fr`) doesn't fall back to a
    // `<length-percentage>` either, since `fr` isn't a length unit.
    assert_eq!(parse("-1fr", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_zero_mantissa_huge_exponent_flex_resolves_to_zero() {
    // `0e999fr` is a zero-mantissa, huge-exponent literal that cssparser's
    // tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section is canonical for the
    // mechanism), but `next_numeric_stable` (which `parse_grid_flex_res`
    // acquires its token through) corrects that before the `is_finite()`
    // guard ever runs. So `0e999fr` resolves to the spec-correct
    // `GridTrackBreadth::Flex(0.0)` directly, per CSS Syntax 3 §4.3.13
    // (`true value is 0`) and CSS Grid 1 §7.2.4 (`<flex [0,∞]>` allows 0).
    assert_eq!(
        parse("0e999fr", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Flex(0.0)
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_fit_content() {
    assert_eq!(
        parse("fit-content(40%)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::FitContent(
                    Length::Percent(40.0)
                ))],
            }))
        ))
    );
}

#[test]
fn grid_template_columns_fit_content_rejects_negative_length() {
    // `fit-content( <length-percentage [0,∞]> )` (CSS Grid Layout
    // Module Level 1 §7.2.1) — `parse_grid_fit_content_res` parses the
    // length itself first (which does accept a negative sign), then
    // rejects it in a separate non-negative check.
    assert_eq!(parse("fit-content(-10px)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_named_lines() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "[full-start] 1fr [content-start] 2fr [content-end] 1fr [full-end]",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(list.components.len(), 3);
    assert_eq!(
        list.line_names,
        vec![
            vec![SmolStr::new("full-start")],
            vec![SmolStr::new("content-start")],
            vec![SmolStr::new("content-end")],
            vec![SmolStr::new("full-end")],
        ]
    );
}

#[test]
fn grid_template_columns_named_lines_reject_span_and_auto() {
    // CSS Grid Layout Module Level 1 §7.2.2 verbatim: "A line name
    // cannot be span or auto, i.e. the `<custom-ident>` in the
    // `<line-names>` production excludes the keywords span and auto."
    // `parse_line_names` must reject these the same way
    // `parse_grid_custom_ident` already does for `<grid-line>`
    // productions — a malformed `<line-names>` invalidates the whole
    // declaration (silent drop), same as any other grammar violation.
    assert_eq!(parse("[auto] 1fr", "grid-template-columns"), None);
    assert_eq!(parse("[span] 1fr", "grid-template-columns"), None);
    // Case-insensitive, same as `is_reserved_grid_line_name`.
    assert_eq!(parse("[AUTO] 1fr", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_repeat_integer() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) =
        parse("repeat(3, 1fr)", "grid-template-columns")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![GridTrackListComponent::Repeat(GridTrackRepeat {
            count: GridRepeatCount::Count(3),
            line_names: vec![vec![], vec![]],
            tracks: vec![GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0))],
        })]
    );
}

#[test]
fn grid_template_columns_repeat_auto_fill() {
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) = parse(
        "repeat(auto-fill, minmax(100px, 1fr))",
        "grid-template-columns",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(
        list.components,
        vec![GridTrackListComponent::Repeat(GridTrackRepeat {
            count: GridRepeatCount::AutoFill,
            line_names: vec![vec![], vec![]],
            tracks: vec![GridTrackSize::MinMax(
                GridInflexibleBreadth::Length(Length::Px(100.0)),
                GridTrackBreadth::Flex(1.0),
            )],
        })]
    );
}

#[test]
fn grid_template_columns_repeat_auto_fit() {
    assert!(matches!(
        parse("repeat(auto-fit, 100px)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
}

#[test]
fn grid_template_columns_repeat_auto_fill_rejects_flex() {
    // CSS Grid Layout Module Level 1 §7.2.3.1 verbatim: "Automatic
    // repetitions (auto-fill or auto-fit) cannot be combined with fully
    // intrinsic or flexible sizes" — `<auto-repeat>` requires
    // `<fixed-size>`, which excludes bare `fr`.
    assert_eq!(
        parse("repeat(auto-fill, 1fr)", "grid-template-columns"),
        None
    );
}

#[test]
fn grid_template_columns_repeat_auto_fill_rejects_bare_min_content() {
    // `<fixed-size>` also excludes bare `min-content`/`max-content`/
    // `auto` (only `<fixed-breadth>`, or `minmax()` with a
    // `<fixed-breadth>` side, qualify).
    assert_eq!(
        parse("repeat(auto-fill, min-content)", "grid-template-columns"),
        None
    );
}

#[test]
fn grid_template_columns_rejects_second_auto_repeat() {
    // §7.2.3.1 verbatim: "It can only appear once in the track list".
    assert_eq!(
        parse(
            "repeat(auto-fill, 100px) repeat(auto-fit, 100px)",
            "grid-template-columns"
        ),
        None
    );
}

#[test]
fn grid_template_columns_allows_auto_repeat_plus_fixed_repeat() {
    // §7.2.3.1 verbatim: "...but the same track list can also contain
    // `<fixed-repeat>`s." — a numeric `repeat()` alongside the one
    // `auto-fill`/`auto-fit` is valid provided its own tracks are also
    // `<fixed-size>`.
    assert!(matches!(
        parse(
            "repeat(2, 50px) repeat(auto-fill, 100px)",
            "grid-template-columns"
        ),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
}

#[test]
fn grid_template_columns_rejects_zero_repeat_count() {
    assert_eq!(parse("repeat(0, 1fr)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_rejects_repeat_with_no_tracks() {
    // `repeat( <count>, [ <line-names>? <track-size> ]+ <line-names>? )`
    // — the `+` requires at least 1 track; a bare trailing comma with
    // nothing after it parses the count and comma but finds no
    // `<track-size>`.
    assert_eq!(parse("repeat(3,)", "grid-template-columns"), None);
}

#[test]
fn grid_template_columns_auto_repeat_plus_bare_track_validates_together() {
    // `grid_track_list_obeys_auto_repeat_constraint`'s fixed-size walk
    // must check bare `Size` components too, not just the tracks
    // nested inside `repeat()` —
    // `grid_template_columns_allows_auto_repeat_plus_fixed_repeat` above
    // only combines 2 `repeat()`s, never a bare `Size` component
    // alongside an auto-repeat.
    assert!(matches!(
        parse("100px repeat(auto-fill, 50px)", "grid-template-columns"),
        Some(PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(_)
        ))
    ));
    // `fit-content()` has no `<fixed-size>` alternative
    // (`grid_track_size_is_fixed` doc) — a bare `fit-content()`
    // component alongside an auto-repeat makes the whole declaration
    // invalid.
    assert_eq!(
        parse(
            "fit-content(50%) repeat(auto-fill, 50px)",
            "grid-template-columns"
        ),
        None
    );
}

#[test]
fn grid_template_columns_rejects_unknown_ident() {
    assert_eq!(parse("bogus", "grid-template-columns"), None);
}

#[test]
fn grid_template_rows_shares_the_same_grammar() {
    assert_eq!(
        parse("50%", "grid-template-rows"),
        Some(PropertyValue::GridTemplateRows(GridTemplateTracks::List(
            Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                    GridTrackBreadth::Length(Length::Percent(50.0))
                ))],
            })
        )))
    );
}

// ── grid-template-areas (CSS Grid Layout Module Level 1 §7.3) ───────

#[test]
fn grid_template_areas_none() {
    assert_eq!(
        parse("none", "grid-template-areas"),
        Some(PropertyValue::GridTemplateAreas(
            GridTemplateAreasValue::None
        ))
    );
}

#[test]
fn grid_template_areas_simple_single_cell() {
    assert_eq!(
        parse(r#""a""#, "grid-template-areas"),
        Some(PropertyValue::GridTemplateAreas(
            GridTemplateAreasValue::Areas(Arc::new(GridTemplateAreas {
                row_strings: vec![SmolStr::new("a")],
                areas: vec![GridTemplateAreaEntry {
                    name: SmolStr::new("a"),
                    row_start: 1,
                    row_end: 2,
                    column_start: 1,
                    column_end: 2,
                }],
                row_count: 1,
                column_count: 1,
            }))
        ))
    );
}

#[test]
fn grid_template_areas_multi_row_multi_col_with_null_cells() {
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) = parse(
        r#""header header" "nav main" "footer ...""#,
        "grid-template-areas",
    ) else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected parsed areas");
    };
    assert_eq!(areas.row_count, 3);
    assert_eq!(areas.column_count, 2);
    let mut names: Vec<&str> = areas.areas.iter().map(|a| a.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["footer", "header", "main", "nav"]);
    let header = areas.areas.iter().find(|a| a.name == "header").unwrap();
    assert_eq!(
        (
            header.row_start,
            header.row_end,
            header.column_start,
            header.column_end
        ),
        (1, 2, 1, 3)
    );
    let footer = areas.areas.iter().find(|a| a.name == "footer").unwrap();
    // `...` is a run of `.` null-cell tokens spanning the second
    // column — `footer` only occupies the first column of row 3.
    assert_eq!(
        (
            footer.row_start,
            footer.row_end,
            footer.column_start,
            footer.column_end
        ),
        (3, 4, 1, 2)
    );
}

#[test]
fn grid_template_areas_spanning_area_forms_rectangle() {
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) =
        parse(r#""a a" "a a""#, "grid-template-areas")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected parsed areas");
    };
    assert_eq!(areas.areas.len(), 1);
    let a = &areas.areas[0];
    assert_eq!(
        (a.row_start, a.row_end, a.column_start, a.column_end),
        (1, 3, 1, 3)
    );
}

#[test]
fn grid_template_areas_rejects_uneven_columns() {
    // spec verbatim: "All strings must define the same number of cell
    // tokens ... or else the declaration is invalid."
    assert_eq!(parse(r#""a b" "c""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_non_rectangular_area() {
    // spec verbatim: "If a named grid area spans multiple grid cells,
    // but those cells do not form a single filled-in rectangle, the
    // declaration is invalid."
    assert_eq!(parse(r#""a b" "b a""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_trash_token() {
    // spec verbatim: "A trash token is a syntax error, and makes the
    // declaration invalid." `#` is neither an ident code point nor `.`.
    assert_eq!(parse(r#""a #""#, "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_treats_non_ascii_space_as_an_ident_char_not_whitespace() {
    // CSS Syntax 3 whitespace is exactly {tab, newline, space}
    // (<https://www.w3.org/TR/css-syntax-3/#whitespace>) — U+3000
    // IDEOGRAPHIC SPACE is not whitespace under that definition, so it
    // must fall into the "ident code point" bucket (CSS Syntax 3's
    // ident-code-point production includes any non-ASCII code point),
    // becoming *part of* the named-cell token rather than a silently
    // skipped separator between two cells.
    let Some(PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas))) =
        parse("\"a\u{3000}b\"", "grid-template-areas")
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a GridTemplateAreas value");
    };
    // A single named cell "a\u{3000}b", not two cells "a" and "b".
    assert_eq!(areas.row_count, 1);
    assert_eq!(areas.column_count, 1);
}

#[test]
fn grid_template_areas_rejects_vertical_tab_as_trash_not_whitespace() {
    // U+000B LINE TABULATION is Unicode `White_Space` but not CSS
    // whitespace (only tab/newline/space qualify) and not an ASCII
    // ident code point either, so it must be a trash token — same
    // shape as `grid_template_areas_rejects_trash_token`.
    assert_eq!(parse("\"a\u{b}b\"", "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_a_value_with_no_strings_at_all() {
    // `none | <string>+` — neither the `none` keyword nor any `<string>`
    // is present, so `parse_grid_template_areas`'s `rows.is_empty()`
    // guard rejects it before ever reaching `build_grid_template_areas`
    // (distinct from `grid_template_areas_rejects_trash_token` above,
    // where a string IS present but its content is invalid).
    assert_eq!(parse("5px", "grid-template-areas"), None);
}

#[test]
fn grid_template_areas_rejects_empty_string_list() {
    assert_eq!(parse(r#""""#, "grid-template-areas"), None);
}

// ── grid-auto-columns / grid-auto-rows (CSS Grid Layout Module Level 1
//    §7.6) ──────────────────────────────────────────────────────────

#[test]
fn grid_auto_columns_single_track() {
    assert_eq!(
        parse("200px", "grid-auto-columns"),
        Some(PropertyValue::GridAutoColumns(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::Length(Length::Px(200.0)))
        ])))
    );
}

#[test]
fn grid_auto_columns_multiple_tracks() {
    assert_eq!(
        parse("100px 1fr", "grid-auto-columns"),
        Some(PropertyValue::GridAutoColumns(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::Length(Length::Px(100.0))),
            GridTrackSize::Breadth(GridTrackBreadth::Flex(1.0)),
        ])))
    );
}

#[test]
fn grid_auto_rows_shares_the_same_grammar() {
    assert_eq!(
        parse("min-content", "grid-auto-rows"),
        Some(PropertyValue::GridAutoRows(Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::MinContent)
        ])))
    );
}

#[test]
fn grid_auto_columns_rejects_repeat() {
    // `<track-size>+` — `repeat()` is not part of this grammar (unlike
    // `grid-template-columns`'s `<track-list>`).
    assert_eq!(parse("repeat(2, 10px)", "grid-auto-columns"), None);
}

// ── grid-auto-flow (CSS Grid Layout Module Level 1 §7.7) ────────────

#[test]
fn grid_auto_flow_row() {
    assert_eq!(
        parse("row", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::Row))
    );
}

#[test]
fn grid_auto_flow_column() {
    assert_eq!(
        parse("column", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::Column))
    );
}

#[test]
fn grid_auto_flow_dense_alone_defaults_to_row() {
    assert_eq!(
        parse("dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
}

#[test]
fn grid_auto_flow_row_dense_either_order() {
    assert_eq!(
        parse("row dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
    assert_eq!(
        parse("dense row", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::RowDense))
    );
}

#[test]
fn grid_auto_flow_column_dense() {
    assert_eq!(
        parse("column dense", "grid-auto-flow"),
        Some(PropertyValue::GridAutoFlow(GridAutoFlowValue::ColumnDense))
    );
}

#[test]
fn grid_auto_flow_rejects_empty() {
    assert_eq!(parse("", "grid-auto-flow"), None);
}

// ── grid-row-start / grid-row-end / grid-column-start /
//    grid-column-end (CSS Grid Layout Module Level 1 §8.3) ──────────

#[test]
fn grid_line_auto() {
    assert_eq!(
        parse("auto", "grid-row-start"),
        Some(PropertyValue::GridRowStart(GridLineValue::Auto))
    );
}

#[test]
fn grid_line_positive_and_negative_integer() {
    assert_eq!(
        parse("3", "grid-row-start"),
        Some(PropertyValue::GridRowStart(GridLineValue::Line(3)))
    );
    assert_eq!(
        parse("-1", "grid-row-end"),
        Some(PropertyValue::GridRowEnd(GridLineValue::Line(-1)))
    );
}

#[test]
fn grid_line_rejects_zero() {
    // spec verbatim: "Negative integers or zero are invalid."
    assert_eq!(parse("0", "grid-column-start"), None);
}

#[test]
fn grid_line_bare_custom_ident() {
    assert_eq!(
        parse("content-start", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::Named(
            SmolStr::new("content-start")
        )))
    );
}

#[test]
fn grid_line_integer_and_name_either_order() {
    assert_eq!(
        parse("2 content-start", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::NamedLine(
            SmolStr::new("content-start"),
            2
        )))
    );
    assert_eq!(
        parse("content-start 2", "grid-column-start"),
        Some(PropertyValue::GridColumnStart(GridLineValue::NamedLine(
            SmolStr::new("content-start"),
            2
        )))
    );
}

#[test]
fn grid_line_span_integer() {
    assert_eq!(
        parse("span 3", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::Span(3)))
    );
}

#[test]
fn grid_line_span_rejects_zero_or_negative() {
    assert_eq!(parse("span 0", "grid-column-end"), None);
    assert_eq!(parse("span -1", "grid-column-end"), None);
}

#[test]
fn grid_line_span_named() {
    assert_eq!(
        parse("span content-end", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            1
        )))
    );
}

#[test]
fn grid_line_span_named_and_integer_either_order() {
    assert_eq!(
        parse("span 2 content-end", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            2
        )))
    );
    assert_eq!(
        parse("span content-end 2", "grid-column-end"),
        Some(PropertyValue::GridColumnEnd(GridLineValue::SpanNamed(
            SmolStr::new("content-end"),
            2
        )))
    );
}

#[test]
fn grid_line_span_alone_is_invalid() {
    // grammar: `span && [ <integer> || <custom-ident> ]` — the bracketed
    // group is mandatory.
    assert_eq!(parse("span", "grid-row-start"), None);
}

#[test]
fn grid_line_rejects_span_and_auto_as_custom_ident() {
    // spec verbatim (§8.3): "the `<custom-ident>` additionally excludes
    // the keywords `span` and `auto`".
    assert_eq!(parse("span", "grid-column-start"), None);
}

#[test]
fn grid_line_integer_then_reserved_ident_leaves_leftover_for_caller_exhausted_check() {
    // Sibling of `grid_line_rejects_span_and_auto_as_custom_ident` above,
    // but exercised through the plain `<integer> <custom-ident>`
    // alternative instead of the `span` prefix: after the leading `2`
    // is consumed, `auto` fails the trailing `<custom-ident>`
    // alternative (same additional exclusion) and is left unconsumed.
    // Same "helper returns `Some`, rejection is the caller's job"
    // pattern as
    // `text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check`.
    let mut input = ParserInput::new("2 auto");
    let mut parser = Parser::new(&mut input);
    assert_eq!(parse_grid_line(&mut parser), Some(GridLineValue::Line(2)));
    assert!(!parser.is_exhausted());
}

#[test]
fn grid_line_rejects_a_value_that_is_neither_integer_nor_ident() {
    // `[ [ <integer> ] && <custom-ident>? ] | <custom-ident> | auto`
    // (`GridLineValue` doc) — a `<string>` token satisfies none of the
    // alternatives `parse_grid_line` tries (`auto`, `span`, `<integer>`,
    // `<custom-ident>`), so it's rejected outright rather than leaving
    // a leftover token.
    assert_eq!(parse(r#""foo""#, "grid-row-start"), None);
}

// ── grid-row / grid-column shorthand (CSS Grid Layout Module Level 1
//    §8.4) ───────────────────────────────────────────────────────────

#[test]
fn grid_row_shorthand_two_values() {
    assert_eq!(
        parse("2 / 5", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Line(2),
            end: GridLineValue::Line(5),
        }))
    );
}

#[test]
fn grid_row_shorthand_omitted_second_copies_custom_ident() {
    // spec verbatim: "if the first value is a `<custom-ident>`, the
    // grid-row-end / grid-column-end longhand is also set to that
    // `<custom-ident>`".
    assert_eq!(
        parse("content", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Named(SmolStr::new("content")),
            end: GridLineValue::Named(SmolStr::new("content")),
        }))
    );
}

#[test]
fn grid_row_shorthand_omitted_second_defaults_to_auto_for_non_ident() {
    // spec verbatim: "...otherwise, it is set to auto."
    assert_eq!(
        parse("3", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Line(3),
            end: GridLineValue::Auto,
        }))
    );
    assert_eq!(
        parse("span 2", "grid-row"),
        Some(PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Span(2),
            end: GridLineValue::Auto,
        }))
    );
}

#[test]
fn grid_column_shorthand_two_values() {
    assert_eq!(
        parse("main-start / main-end", "grid-column"),
        Some(PropertyValue::GridColumn(GridLineShorthand {
            start: GridLineValue::Named(SmolStr::new("main-start")),
            end: GridLineValue::Named(SmolStr::new("main-end")),
        }))
    );
}

// ── justify-items / justify-self (CSS Box Alignment Module Level 3
//    §7.1 / §6.1) ────────────────────────────────────────────────────

#[test]
fn justify_items_parse_keywords() {
    assert_eq!(
        parse("center", "justify-items"),
        Some(PropertyValue::JustifyItems(SelfAlignmentValue::Center))
    );
    assert_eq!(
        parse("stretch", "justify-items"),
        Some(PropertyValue::JustifyItems(SelfAlignmentValue::Stretch))
    );
}

#[test]
fn justify_self_parse_auto_and_keywords() {
    assert_eq!(
        parse("auto", "justify-self"),
        Some(PropertyValue::JustifySelf(AlignSelfValue::Auto))
    );
    assert_eq!(
        parse("end", "justify-self"),
        Some(PropertyValue::JustifySelf(AlignSelfValue::Value(
            SelfAlignmentValue::End
        )))
    );
}

// ── place-items / place-self (CSS Box Alignment Module Level 3 §7.3 /
//    §6.3) ────────────────────────────────────────────────────────────

#[test]
fn place_items_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("center", "place-items"),
        Some(PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Center,
            justify: SelfAlignmentValue::Center,
        }))
    );
}

#[test]
fn place_items_shorthand_two_values() {
    assert_eq!(
        parse("start end", "place-items"),
        Some(PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Start,
            justify: SelfAlignmentValue::End,
        }))
    );
}

#[test]
fn place_self_shorthand_single_value_copies_to_both() {
    assert_eq!(
        parse("auto", "place-self"),
        Some(PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Auto,
        }))
    );
}

#[test]
fn place_self_shorthand_two_values() {
    assert_eq!(
        parse("center stretch", "place-self"),
        Some(PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Value(SelfAlignmentValue::Center),
            justify: AlignSelfValue::Value(SelfAlignmentValue::Stretch),
        }))
    );
}

// ── key() discriminant integrity (mirrors `flex_group_keys_map_correctly`) ─

#[test]
fn grid_group_keys_map_correctly() {
    assert_eq!(
        PropertyValue::GridTemplateColumns(GridTemplateTracks::None).key(),
        PropertyKey::GridTemplateColumns
    );
    assert_eq!(
        PropertyValue::GridTemplateRows(GridTemplateTracks::None).key(),
        PropertyKey::GridTemplateRows
    );
    assert_eq!(
        PropertyValue::GridTemplateAreas(GridTemplateAreasValue::None).key(),
        PropertyKey::GridTemplateAreas
    );
    assert_eq!(
        PropertyValue::GridAutoColumns(Arc::new(vec![GridTrackSize::Breadth(
            GridTrackBreadth::Auto
        )]))
        .key(),
        PropertyKey::GridAutoColumns
    );
    assert_eq!(
        PropertyValue::GridAutoRows(Arc::new(vec![GridTrackSize::Breadth(
            GridTrackBreadth::Auto
        )]))
        .key(),
        PropertyKey::GridAutoRows
    );
    assert_eq!(
        PropertyValue::GridAutoFlow(GridAutoFlowValue::Row).key(),
        PropertyKey::GridAutoFlow
    );
    assert_eq!(
        PropertyValue::GridRowStart(GridLineValue::Auto).key(),
        PropertyKey::GridRowStart
    );
    assert_eq!(
        PropertyValue::GridRowEnd(GridLineValue::Auto).key(),
        PropertyKey::GridRowEnd
    );
    assert_eq!(
        PropertyValue::GridColumnStart(GridLineValue::Auto).key(),
        PropertyKey::GridColumnStart
    );
    assert_eq!(
        PropertyValue::GridColumnEnd(GridLineValue::Auto).key(),
        PropertyKey::GridColumnEnd
    );
    assert_eq!(
        PropertyValue::GridRow(GridLineShorthand {
            start: GridLineValue::Auto,
            end: GridLineValue::Auto,
        })
        .key(),
        PropertyKey::GridRow
    );
    assert_eq!(
        PropertyValue::GridColumn(GridLineShorthand {
            start: GridLineValue::Auto,
            end: GridLineValue::Auto,
        })
        .key(),
        PropertyKey::GridColumn
    );
    assert_eq!(
        PropertyValue::JustifyItems(SelfAlignmentValue::Normal).key(),
        PropertyKey::JustifyItems
    );
    assert_eq!(
        PropertyValue::JustifySelf(AlignSelfValue::Auto).key(),
        PropertyKey::JustifySelf
    );
    assert_eq!(
        PropertyValue::PlaceItems(PlaceItemsShorthand {
            align: SelfAlignmentValue::Normal,
            justify: SelfAlignmentValue::Normal,
        })
        .key(),
        PropertyKey::PlaceItems
    );
    assert_eq!(
        PropertyValue::PlaceSelf(PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Auto,
        })
        .key(),
        PropertyKey::PlaceSelf
    );
}

// ── orphans / widows (CSS Fragmentation Module Level 3 §3.3) ──
//
// Value grammar: `<integer>`, restricted to positive integers by spec
// prose ("Only positive integers are allowed as values of orphans and
// widows. Negative values and zero are invalid and must cause the
// declaration to be ignored."). Initial: 2 / Inherited: yes / Computed
// value: specified integer.

#[test]
fn orphans_widows_parse_valid_positive_integers() {
    assert_eq!(parse("1", "orphans"), Some(PropertyValue::Orphans(1)));
    assert_eq!(parse("2", "orphans"), Some(PropertyValue::Orphans(2)));
    assert_eq!(parse("100", "orphans"), Some(PropertyValue::Orphans(100)));
    assert_eq!(parse("1", "widows"), Some(PropertyValue::Widows(1)));
    assert_eq!(parse("3", "widows"), Some(PropertyValue::Widows(3)));
}

#[test]
fn orphans_widows_parse_leading_plus_sign() {
    // CSS Values and Units 3 §4.2 `<integer>`: a leading `+` sign is
    // part of the grammar (`[+-]? digit+`), not an error.
    assert_eq!(parse("+3", "orphans"), Some(PropertyValue::Orphans(3)));
    assert_eq!(parse("+3", "widows"), Some(PropertyValue::Widows(3)));
}

#[test]
fn orphans_widows_reject_zero() {
    // Spec verbatim: "Negative values and zero are invalid and must
    // cause the declaration to be ignored."
    assert_eq!(parse("0", "orphans"), None);
    assert_eq!(parse("0", "widows"), None);
}

#[test]
fn orphans_widows_reject_negative_integers() {
    assert_eq!(parse("-1", "orphans"), None);
    assert_eq!(parse("-100", "orphans"), None);
    assert_eq!(parse("-1", "widows"), None);
}

#[test]
fn orphans_widows_reject_non_integer_values() {
    // Fractional numbers, lengths, idents, and strings are all outside
    // the `<integer>` grammar.
    assert_eq!(parse("1.5", "orphans"), None);
    assert_eq!(parse("2px", "orphans"), None);
    assert_eq!(parse("auto", "orphans"), None);
    assert_eq!(parse(r#""2""#, "orphans"), None);
    assert_eq!(parse("1.5", "widows"), None);
    assert_eq!(parse("2px", "widows"), None);
}

#[test]
fn orphans_widows_reject_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future
    // work), silent drop (`PropertyValue` doc's "CSS-wide keyword"
    // section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "orphans"), None);
        assert_eq!(parse(kw, "widows"), None);
    }
}

#[test]
fn orphans_widows_key_maps_to_distinct_property_keys() {
    assert_eq!(PropertyValue::Orphans(2).key(), PropertyKey::Orphans);
    assert_eq!(PropertyValue::Widows(2).key(), PropertyKey::Widows);
}

// ── table-layout (CSS Tables 3 §4) ────────────────────────────────────
//
// WPT css/css-tables/parsing/table-layout-{valid,invalid}.html の
// grammar (`auto | fixed`) を check する。invalid 側 2 case
// (`none` / `auto fixed`) は caller の `expect_exhausted`
// (rule.rs::DeclParser) が落とす — ここでは `parse_entire` で同条件を
// 再現する。

#[test]
fn table_layout_accepts_auto_and_fixed() {
    assert_eq!(
        parse("auto", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Auto))
    );
    assert_eq!(
        parse("fixed", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Fixed))
    );
    // ASCII case-insensitive (CSS Values 3 §3.1)。
    assert_eq!(
        parse("FIXED", "table-layout"),
        Some(PropertyValue::TableLayout(TableLayoutValue::Fixed))
    );
}

#[test]
fn table_layout_rejects_invalid() {
    assert_eq!(parse("none", "table-layout"), None);
    assert_eq!(parse_entire("auto fixed", "table-layout"), None);
    assert_eq!(parse("collapse", "table-layout"), None);
}

#[test]
fn table_layout_key_maps_to_table_layout_property_key() {
    let v = PropertyValue::TableLayout(TableLayoutValue::Auto);
    assert_eq!(v.key(), PropertyKey::TableLayout);
}

// ── border-collapse (CSS Tables 3 §6) ─────────────────────────────────
//
// WPT css/css-tables/parsing/border-collapse-{valid,invalid}.html の
// grammar (`collapse | separate`) を check する (同上の構成)。

#[test]
fn border_collapse_accepts_collapse_and_separate() {
    assert_eq!(
        parse("collapse", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Collapse))
    );
    assert_eq!(
        parse("separate", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Separate))
    );
    // ASCII case-insensitive (CSS Values 3 §3.1)。
    assert_eq!(
        parse("COLLAPSE", "border-collapse"),
        Some(PropertyValue::BorderCollapse(BorderCollapseValue::Collapse))
    );
}

#[test]
fn border_collapse_rejects_invalid() {
    assert_eq!(parse("none", "border-collapse"), None);
    assert_eq!(parse_entire("separate collapse", "border-collapse"), None);
    assert_eq!(parse("fixed", "border-collapse"), None);
}

#[test]
fn border_collapse_key_maps_to_border_collapse_property_key() {
    let v = PropertyValue::BorderCollapse(BorderCollapseValue::Separate);
    assert_eq!(v.key(), PropertyKey::BorderCollapse);
}

// ── border-spacing (CSS Tables 3 §6.1) ─────────────────────────────────
//
// WPT css/css-tables/parsing/border-spacing-{valid,invalid}.html の
// 全 case の pin。valid の `calc()` 混じり 2 件は上流 deferred path が
// `Deferred` に回す (`width: calc(..)` と同型) ため、ここでは受理
// (`Some`) のみ assert し payload の中身は assert しない。

#[test]
fn border_spacing_accepts_valid_values() {
    // 単一 standard length は両軸に double する (spec 本文 +
    // `GapShorthand` と同型)。
    assert_eq!(
        parse("0px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(0.0),
            vertical: Length::Px(0.0),
        }))
    );
    // 2 成分。
    assert_eq!(
        parse("10px 20px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(10.0),
            vertical: Length::Px(20.0),
        }))
    );
    // unitless `0` は `<length>` として受理 (WPT computed の `"0"` case)。
    assert_eq!(
        parse("0", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Px(0.0),
            vertical: Length::Px(0.0),
        }))
    );
    // font-relative も plain length として受理。
    assert_eq!(
        parse("0.5em 1px", "border-spacing"),
        Some(PropertyValue::BorderSpacing(BorderSpacingValue {
            horizontal: Length::Em(0.5),
            vertical: Length::Px(1.0),
        }))
    );
    // `calc()` 混じりは deferred path が受理する (payload は `Deferred`)。
    let deferred = parse_entire("calc(10px + 0.5em) calc(10px - 0.5em)", "border-spacing");
    assert!(
        matches!(deferred, Some(PropertyValue::Deferred(_))),
        "calc border-spacing should defer, got {deferred:?}"
    );
    // `calc()` 単一値も同様。
    assert!(matches!(
        parse_entire("calc(10px + 0.5em)", "border-spacing"),
        Some(PropertyValue::Deferred(_))
    ));
    // key mapping。
    let v = PropertyValue::BorderSpacing(BorderSpacingValue {
        horizontal: Length::Px(1.0),
        vertical: Length::Px(2.0),
    });
    assert_eq!(v.key(), PropertyKey::BorderSpacing);
}

#[test]
fn border_spacing_rejects_invalid_values() {
    // `<percentage>` は Percentages: N/A のため reject。
    assert_eq!(parse("10%", "border-spacing"), None);
    // 負 length は illegal のため reject。
    assert_eq!(parse("-20px", "border-spacing"), None);
    assert_eq!(parse_entire("10px -20px", "border-spacing"), None);
    // bare non-zero number は `<length>` ではないため reject。
    assert_eq!(parse("30", "border-spacing"), None);
    // 3 成分は `{1,2}` を満たさないため reject。
    assert_eq!(parse_entire("40px 50px 60px", "border-spacing"), None);
    // keyword は `<length>` ではないため reject。
    assert_eq!(parse("auto", "border-spacing"), None);
    // `%` 混じり calc は deferred path の guard が reject
    // (`tab-size` の Percentages: N/A guard と同型)。
    assert_eq!(parse_entire("calc(10% + 5px)", "border-spacing"), None);
}

// ── caption-side (CSS Tables 3 §7) ─────────────────────────────────────
//
// WPT css/css-tables/parsing/caption-side-{valid,invalid}.html の
// 全 case の pin。

#[test]
fn caption_side_accepts_valid_values() {
    assert_eq!(
        parse("top", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Top))
    );
    assert_eq!(
        parse("bottom", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Bottom))
    );
    // ASCII case-insensitive (`table-layout` の `FIXED` case と同型)。
    assert_eq!(
        parse("TOP", "caption-side"),
        Some(PropertyValue::CaptionSide(CaptionSideValue::Top))
    );
    let v = PropertyValue::CaptionSide(CaptionSideValue::Top);
    assert_eq!(v.key(), PropertyKey::CaptionSide);
}

#[test]
fn caption_side_rejects_invalid_values() {
    assert_eq!(parse("auto", "caption-side"), None);
    assert_eq!(parse("left", "caption-side"), None);
    assert_eq!(parse("right", "caption-side"), None);
    assert_eq!(parse_entire("top bottom", "caption-side"), None);
    assert_eq!(parse("10px", "caption-side"), None);
}

// ── empty-cells (CSS Tables 3 §8) ──────────────────────────────────────
//
// WPT css/css-tables/parsing/empty-cells-{valid,invalid}.html の
// 全 case の pin。

#[test]
fn empty_cells_accepts_valid_values() {
    assert_eq!(
        parse("show", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Show))
    );
    assert_eq!(
        parse("hide", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Hide))
    );
    // ASCII case-insensitive (`table-layout` の `FIXED` case と同型)。
    assert_eq!(
        parse("HIDE", "empty-cells"),
        Some(PropertyValue::EmptyCells(EmptyCellsValue::Hide))
    );
    let v = PropertyValue::EmptyCells(EmptyCellsValue::Show);
    assert_eq!(v.key(), PropertyKey::EmptyCells);
}

#[test]
fn empty_cells_rejects_invalid_values() {
    assert_eq!(parse("auto", "empty-cells"), None);
    assert_eq!(parse_entire("show hide", "empty-cells"), None);
    assert_eq!(parse("visible", "empty-cells"), None);
}

#[test]
fn multicol_longhands_parse_and_reject_out_of_range_values() {
    assert_eq!(
        parse_entire("auto", "column-count"),
        Some(PropertyValue::ColumnCount(ColumnCountValue::Auto))
    );
    assert_eq!(
        parse_entire("3", "column-count"),
        Some(PropertyValue::ColumnCount(ColumnCountValue::Count(3)))
    );
    for source in ["0", "-1", "1.5", "3.0"] {
        assert_eq!(parse_entire(source, "column-count"), None, "{source}");
    }

    assert_eq!(
        parse_entire("auto", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Auto))
    );
    assert_eq!(
        parse_entire("10px", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Length(
            Length::Px(10.0)
        )))
    );
    assert!(matches!(
        parse_entire("2em", "column-width"),
        Some(PropertyValue::ColumnWidth(ColumnWidthValue::Length(Length::Em(value))))
            if (value - 2.0).abs() < f32::EPSILON
    ));
    for source in ["-1px", "10%"] {
        assert_eq!(parse_entire(source, "column-width"), None, "{source}");
    }
}

#[test]
fn multicol_columns_accepts_both_orders_and_rejects_duplicate_non_auto_components() {
    let expected = |count, width| PropertyValue::Columns(ColumnsShorthand { count, width });
    for source in ["3 100px", "100px 3"] {
        assert_eq!(
            parse_entire(source, "columns"),
            Some(expected(
                ColumnCountValue::Count(3),
                ColumnWidthValue::Length(Length::Px(100.0)),
            )),
            "{source}" // cov:ignore: assertion failure formatting is only evaluated on a regression.
        );
    }
    assert_eq!(
        parse_entire("auto 3", "columns"),
        Some(expected(ColumnCountValue::Count(3), ColumnWidthValue::Auto,))
    );
    assert_eq!(
        parse_entire("auto 200px", "columns"),
        Some(expected(
            ColumnCountValue::Auto,
            ColumnWidthValue::Length(Length::Px(200.0)),
        ))
    );
    assert_eq!(
        parse_entire("auto", "columns"),
        Some(expected(ColumnCountValue::Auto, ColumnWidthValue::Auto))
    );
    assert_eq!(
        parse_entire("3", "columns"),
        Some(expected(ColumnCountValue::Count(3), ColumnWidthValue::Auto,))
    );
    for source in ["3 4", "100px 200px", "3 100px 2"] {
        // cov:ignore: assertion failure formatting is only evaluated on a regression.
        assert_eq!(parse_entire(source, "columns"), None, "{source}");
    }
    assert_eq!(parse_entire("100px 3 2", "columns"), None);
    assert_eq!(
        parse_entire("auto auto", "columns"),
        Some(expected(ColumnCountValue::Auto, ColumnWidthValue::Auto))
    );
}
