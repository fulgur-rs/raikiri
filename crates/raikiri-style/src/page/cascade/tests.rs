//! Verification tests for `@page` cascade order.
//!
//! Spec anchors:
//! - CSS Paged Media L3 §"Cascading and page context" —
//!   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
//! - CSS Cascading L4 §"Cascade Origin" —
//!   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
//!
//! Test naming mirrors `cascade::tests` (sibling convention).
//!
//! **Note on a source of confusion elsewhere**: some prose elsewhere
//! says "Author !important > UA !important > Author normal > UA normal",
//! which contradicts CSS Cascading L4 §"Cascade Origin" (Important order:
//! Author < User < UA — UA-important wins). The correct order
//! (`UA_imp` > `Author_imp`, exact `cascade_rank` values shift as origin
//! tiers are added over time — see that function's doc) is spec-correct
//! and matches `cascade_rank` and the existing style-rule test
//! `cascade::tests::important_ua_beats_important_author_display`.
//! Implementation follows the spec; this test asserts UA `!important` wins.

use super::*;
use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::property::{
    AlignSelfValue, BackgroundAttachment, BackgroundImage, BackgroundRepeat,
    BackgroundRepeatKeyword, BackgroundShorthand, BorderRadius, BoxShadowItem, BoxSizing,
    BreakBetween, BreakInside, ClearValue, ClipPath, ContentAlignmentValue, ContentComponent,
    CssColor, CustomProperty, Direction, DisplayValue, FilterFunction, FlexDirectionValue,
    FlexWrapValue, FloatValue, FontShorthand, FontShorthandSize, FontStyle, FontVariantCaps,
    FontWeightValue, GeometryBox, GridAreaShorthand, GridAutoFlowValue, GridInflexibleBreadth,
    GridLineShorthand, GridLineValue, GridRepeatCount, GridShorthand, GridTemplateAreaEntry,
    GridTemplateAreas, GridTemplateAreasValue, GridTemplateTracks, GridTrackBreadth, GridTrackList,
    GridTrackListComponent, GridTrackRepeat, GridTrackSize, HangingPunctuation, Hyphens, Isolation,
    Length, LengthOrAuto, LengthOrNormal, LineBreak, LineHeight, ListStylePosition, ListStyleType,
    MaskImage, MixBlendMode, ObjectFit, Outline, OutlineStyle, OverflowValue, OverflowWrap,
    OverflowXY, PageValue, PlaceContentShorthand, PlaceItemsShorthand, PlaceSelfShorthand,
    PositionValue, RelativeFontSize, RubyPosition, SelfAlignmentValue, StartEnd, TabSize,
    TextAlign, TextAlignAll, TextAlignLast, TextDecorationColor, TextDecorationInset,
    TextDecorationLine, TextDecorationShorthand, TextDecorationSkipInk, TextDecorationSkipSpaces,
    TextDecorationStyle, TextDecorationThickness, TextEmphasisHEdge, TextEmphasisPosition,
    TextEmphasisVEdge, TextJustify, TextShadowColor, TextTransform, TextUnderlinePosition,
    TransformFunction, VerticalAlign, Visibility, VisualBox, WhiteSpace, WordBreak, WritingMode,
    ZIndexValue,
};
use crate::resolve::{ComputedLength, ComputedLineHeight};
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;
use std::sync::Arc;

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};
const GREEN: CssColor = CssColor {
    r: 0,
    g: 128,
    b: 0,
    a: 255,
};

fn color_of(result: &PageCascadeResult) -> Option<CssColor> {
    match result.declarations().get(&PropertyKey::Color) {
        Some(PropertyValue::Color(c)) => Some(*c),
        _ => None,
    }
}

#[test]
fn cascade_page_empty_rule_tree_returns_empty() {
    let tree = RuleTree::empty();
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert!(result.declarations().is_empty());
}

#[test]
fn cascade_page_default_selector_matches_every_page() {
    // `@page { color: red }` has an empty prelude → matches every page.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_descriptor_size_uses_page_selector_specificity() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { size: 300px 50px }", Origin::Author);
    tree.add_stylesheet("@page :first { size: 400px 60px }", Origin::Author);
    let query = PageContextQuery {
        is_first: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(
        result.size(),
        Some(PageSize::Lengths {
            width: Length::Px(400.0),
            height: Length::Px(60.0),
        })
    );
}

#[test]
fn cascade_page_descriptor_size_uses_important_and_source_order() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { size: 300px !important }", Origin::Author);
    tree.add_stylesheet("@page { size: 400px }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(
        result.size(),
        Some(PageSize::Lengths {
            width: Length::Px(300.0),
            height: Length::Px(300.0),
        })
    );

    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { size: 300px }", Origin::Author);
    tree.add_stylesheet("@page { size: 400px }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(
        result.size(),
        Some(PageSize::Lengths {
            width: Length::Px(400.0),
            height: Length::Px(400.0),
        })
    );
}

#[test]
fn cascade_page_descriptor_marks_bleed_and_margin_boxes_are_exposed() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
            "@page { marks: crop cross; bleed: 6pt; @top-left { content: \"A\" } @top-left { content: \"B\" } }",
            Origin::Author,
        );
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(
        result.marks(),
        Some(PageMarks::Marks {
            crop: true,
            cross: true
        })
    );
    assert_eq!(result.bleed(), Some(PageBleed::Length(Length::Px(8.0))));
    assert_eq!(result.margin_boxes().len(), 2);
    assert_eq!(result.margin_boxes()[0].slot, PageMarginBoxSlot::TopLeft);
    assert_eq!(result.margin_boxes()[1].slot, PageMarginBoxSlot::TopLeft);
    assert_eq!(result.margin_boxes()[0].declarations.len(), 1);
    assert_eq!(result.margin_boxes()[1].declarations.len(), 1);
}

#[test]
fn cascade_page_margin_box_source_order_and_selector_metadata_are_retained() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { @top-left { content: \"A\" } }", Origin::Author);
    tree.add_stylesheet(
        "@page :first { @top-left { content: \"B\" } }",
        Origin::Author,
    );
    let query = PageContextQuery {
        is_first: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    let boxes = result.margin_boxes();
    assert_eq!(boxes.len(), 2);
    assert!(boxes[0].source_order < boxes[1].source_order);
    assert_eq!(boxes[0].specificity, (0, 0, 0));
    assert_eq!(boxes[1].specificity, (0, 1, 0));
    assert_eq!(boxes[0].origin, Origin::Author);
    assert_eq!(boxes[1].origin, Origin::Author);
}

#[test]
fn cascade_page_descriptor_named_and_auto_values_stay_typed() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { size: A4 landscape; bleed: auto }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(
        result.size(),
        Some(PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: Some(PageOrientation::Landscape),
        })
    );
    assert_eq!(result.bleed(), Some(PageBleed::Auto));
}

#[test]
fn cascade_page_descriptor_relative_size_uses_page_font_size() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { font-size: 20px; size: 2em 3em }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(
        result.size(),
        Some(PageSize::Lengths {
            width: Length::Px(40.0),
            height: Length::Px(60.0),
        })
    );
}

#[test]
fn cascade_page_named_rule_does_not_apply_to_unnamed_page() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page my-cover { color: red }", Origin::Author);
    let query = PageContextQuery {
        page_name: None,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert!(color_of(&result).is_none());
}

// ── Verification 1: origin cascade (Author > UA for normal) ─────────────
// Spec: CSS Cascading L4 §"Cascade Origin" — Normal author declarations
// beat normal user-agent declarations.

#[test]
fn cascade_page_author_normal_beats_ua_normal() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red }", Origin::UserAgent);
    tree.add_stylesheet("@page { color: blue }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_author_normal_beats_ua_normal_regardless_of_stylesheet_add_order() {
    // Author added first, UA second — origin rank (not source order) still
    // makes Author win because UA-normal rank < Author-normal rank
    // (`cascade_rank` doc has the exact values).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: blue }", Origin::Author);
    tree.add_stylesheet("@page { color: red }", Origin::UserAgent);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(BLUE));
}

// ── Verification 2: !important reversal (UA-important > Author-important) ─
// Spec: CSS Cascading L4 §"Cascade Origin". See module docstring for the
// task-prose divergence note.

#[test]
fn cascade_page_important_ua_beats_important_author() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red !important }", Origin::UserAgent);
    tree.add_stylesheet("@page { color: blue !important }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_important_author_beats_normal_ua_and_author() {
    // rank(Author, important) > rank(Author, normal) > rank(UA, normal)
    // (`cascade_rank` doc has the exact values).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: green }", Origin::UserAgent);
    tree.add_stylesheet("@page { color: blue }", Origin::Author);
    tree.add_stylesheet("@page { color: red !important }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(RED));
}

// ── Verification 3: pseudo-page specificity (:first > :left) ─────────────
// Spec: `@page :first` = (0,1,0), `@page :left` = (0,0,1). (0,1,0) > (0,0,1).

#[test]
fn cascade_page_first_beats_left_by_specificity() {
    // Query is both :first AND :left (first page happens to also be a
    // left/verso page). Both rules match; :first must win.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page :left { color: red } @page :first { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        is_first: true,
        is_left: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_first_beats_left_regardless_of_source_order() {
    // Reverse source order — specificity (rank tier 2) still dominates
    // source order (rank tier 3).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page :first { color: blue } @page :left { color: red }",
        Origin::Author,
    );
    let query = PageContextQuery {
        is_first: true,
        is_left: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

// ── Verification 4: named ident selectivity (named > unnamed) ────────────
// Spec: `@page named` = (1,0,0), `@page` = (0,0,0). (1,0,0) > (0,0,0).

#[test]
fn cascade_page_named_ident_beats_unnamed() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } @page cover { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("cover")),
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_named_ident_case_sensitive() {
    // Cover (rule) vs cover (query) — <custom-ident> is case-sensitive
    // per CSS Values L4 §4.2, so the rule must NOT match. Falls back
    // to the unnamed rule.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } @page Cover { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("cover")),
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_named_ident_auto_never_matches_reserved_keyword() {
    // CSS Paged Media L3 §"Page selectors" (`#page-selectors`):
    //   "A page type name of auto (ASCII case-insensitive) does not
    //    make the rule invalid, but must never match."
    // Even with `query.page_name = Some("auto")` — the strongest form
    // of the rule where a naive byte-equality compare would match —
    // the `@page auto` rule must be excluded from the cascade, so the
    // unnamed fallback (red) wins over the named-auto rule (blue).
    // This is the "rule was excluded" proof shape.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } @page auto { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("auto")),
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_named_ident_auto_case_insensitive_reserved_keyword() {
    // Same spec sentence, ASCII case-insensitive half: `Auto` and
    // `AUTO` are equally reserved and must never match. Every named
    // rule below is excluded, so the unnamed fallback (red) wins
    // regardless of the query's own casing.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } \
             @page Auto { color: blue } \
             @page AUTO { color: green }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("Auto")),
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(RED));
}

// ── Verification 5: named-page cascade produces named-page declarations ──
// Verification target: `(page_name=Some("landscape_a3"), page_index=0,
// is_first=true)` — the winning declarations must come from the named-page
// rule. Property proxy: `color` (already supported, and cascaded through
// `PageCascadeResult`). `size` is parsed now (`PageRule::size_declarations`
// / `PageSize`) but has no cascade winner-selection wired yet, so it
// isn't a usable proxy for *this* test's specificity-tie-break target;
// `margin` *is* both parsed and cascaded but `color` keeps this test
// focused on selector specificity rather than on length resolution, which
// the phase 3 tests cover. The named-page
// rule wins because its specificity `(1, 1, 0)` beats every non-named
// alternative under the L3 §"Cascading and page context" tuple.

#[test]
fn cascade_page_named_page_first_page_declarations_win_over_alternatives() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } \
             @page :first { color: green } \
             @page landscape_a3:first { color: blue } \
             @page other-page { color: red }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("landscape_a3")),
        is_first: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

// ── Additional coverage: OR over entries, AND over pseudos, source order ─

#[test]
fn cascade_page_comma_list_or_semantics() {
    // `@page :first, :left` — comma-separated list is OR. Only :left
    // matches this query, but the rule still contributes.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page :first, :left { color: blue }", Origin::Author);
    let query = PageContextQuery {
        is_left: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_compound_pseudo_and_semantics_requires_all_match() {
    // `@page :first:left` — compound AND. Query has :first only.
    // Rule does NOT match — falls back to nothing (no rule applies).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page :first:left { color: blue }", Origin::Author);
    let query = PageContextQuery {
        is_first: true,
        is_left: false,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert!(color_of(&result).is_none());
}

#[test]
fn cascade_page_compound_pseudo_matches_when_all_conditions_true() {
    // Same rule, now query has both is_first + is_left.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page :first:left { color: blue }", Origin::Author);
    let query = PageContextQuery {
        is_first: true,
        is_left: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_source_order_tiebreak_later_wins() {
    // Two rules of equal (rank, specificity) — later source_order wins
    // per CSS Cascading L4 §6.1
    // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of
    // Appearance", sibling of style-rule
    // `cascade::tests::source_order_tiebreak_later_wins`.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page :first { color: red } @page :first { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        is_first: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_later_duplicate_in_same_rule_wins() {
    // Sibling of `cascade::tests::later_duplicate_in_same_rule_wins`:
    // within a single rule, later declarations of the same property win.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red; color: blue }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_specificity_examples_from_spec() {
    // Directly encode the spec's own examples:
    //   @page { }        → (0,0,0)
    //   @page :left { }  → (0,0,1)
    //   @page :first { } → (0,1,0)
    //   @page artsy { }  → (1,0,0)
    // Winner order: artsy > :first > :left > default when all match.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } \
             @page :left { color: red } \
             @page :first { color: red } \
             @page artsy { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("artsy")),
        is_first: true,
        is_left: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_blank_pseudo_contributes_to_g_component() {
    // `:blank` counts toward `g` (same tier as `:first`) per L3 spec.
    // `@page :blank` = (0,1,0) > `@page :left` = (0,0,1).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page :left { color: red } @page :blank { color: blue }",
        Origin::Author,
    );
    let query = PageContextQuery {
        is_left: true,
        is_blank: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(BLUE));
}

#[test]
fn cascade_page_right_pseudo_contributes_to_h_component() {
    // `:right` = (0,0,1) — same tier as `:left`. Source order tiebreak.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page :right { color: red }", Origin::Author);
    let query = PageContextQuery {
        is_right: true,
        ..Default::default()
    };
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_specificity_ordering_derived_ord() {
    // Direct assertion on the `PageSpecificity` `Ord` derivation ensures
    // the derive-order (f, g, h) matches the spec's tuple ordering.
    let default_spec = PageSpecificity { f: 0, g: 0, h: 0 };
    let left = PageSpecificity { f: 0, g: 0, h: 1 };
    let first = PageSpecificity { f: 0, g: 1, h: 0 };
    let named = PageSpecificity { f: 1, g: 0, h: 0 };
    assert!(default_spec < left);
    assert!(left < first);
    assert!(first < named);
}

#[test]
fn cascade_page_multiple_properties_all_win_independently() {
    // Regression: winner selection is per-property. Two rules setting
    // different properties both contribute.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } @page { font-weight: 700 }",
        Origin::Author,
    );
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(RED));
    assert_eq!(
        result.declarations().get(&PropertyKey::FontWeight),
        Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)))
    );
}

#[test]
fn cascade_page_margin_box_declarations_do_not_enter_the_page_context_cascade() {
    // `PageCascadeResult`'s doc states the margin-box at-rules are
    // "similarly absent" from a `cascade_page` result — this pins that
    // claim directly. A `@top-left` block declaring `color: blue` must
    // not affect (nor even appear alongside) the page context's own
    // `color: red` — the two are separate declaration bags
    // (`PageRule::declarations` vs `PageRule::margin_box_rules`), and
    // only the former is cascaded by this function. This is the
    // invariant a future margin-box cascade integration must preserve: the
    // page context's own cascade result must stay unaffected by what a
    // nested margin-box at-rule declares.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red; @top-left { color: blue } }",
        Origin::Author,
    );
    // Confirm the `@top-left { color: blue }` block was actually parsed
    // and kept (not silently dropped) before asserting it stays out of
    // the cascade result below — otherwise a regression that dropped
    // margin-box at-rules entirely would make this test pass for the
    // wrong reason.
    assert_eq!(tree.page_rules[0].margin_box_rules.len(), 1);
    assert_eq!(
        tree.page_rules[0].margin_box_rules[0].slot,
        PageMarginBoxSlot::TopLeft
    );
    assert_eq!(tree.page_rules[0].margin_box_rules[0].declarations.len(), 1);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(RED));
}

// ── page context inheritance: relative font-weight resolution ───────────
//
// CSS Paged Media 3 §6 "Page Properties"
// <https://www.w3.org/TR/css-page-3/#page-properties>:
//   "As with elements in the document, both the page context and the margin
//    context have a computed value for every property … page-margin boxes
//    inherit from the page context. The page context inherits from the root
//    element. However, since the previous revision of CSS Paged Media Level
//    3 did not specify this point, an implementation that sets inherited
//    properties in the page context to their initial values (as for the root
//    element) is also conformant …"
//
// Table rows come from CSS Fonts 4 §2.2.1 "Relative Weights"
// <https://www.w3.org/TR/css-fonts-4/#relative-weights>; the table itself is
// pinned exhaustively by `cascade::tests::
// font_weight_bolder_lighter_table_all_six_rows`. What these tests check is the
// **integration**: that `cascade_page` resolves against its `PageInheritance`
// argument at all, and which weight it uses as the inherited value
// (before
// this was wired up, `FontWeightValue::Bolder` parked unresolved in the public
// `declarations` map).

/// The two inheritance variants produce distinct results for relative
/// font weights under the same page rule.
#[test]
fn cascade_page_from_root_and_legacy_initial_values_diverge_for_relative_font_weight() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { font-weight: bolder }", Origin::Author);
    let root = root_with_weight(700.0);

    let from_root = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    let legacy = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );

    assert_eq!(
        from_root.declarations().get(&PropertyKey::FontWeight),
        Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
        "`FromRoot(700)` must resolve `bolder` against the supplied root weight"
    );
    assert_eq!(
        legacy.declarations().get(&PropertyKey::FontWeight),
        Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))),
        "`LegacyInitialValues` must resolve `bolder` against the initial \
             weight (400), not the root's 700"
    );
}

/// Root [`ComputedValues`] with `font-weight: w`, everything else initial.
fn root_with_weight(w: f32) -> ComputedValues {
    ComputedValues {
        font_weight: w,
        ..ComputedValues::initial()
    }
}

/// Cascade `@page { font-weight: <decl> }` with `inheritance` as the page
/// context's inheritance parent and return the resolved absolute weight
/// from `declarations`. `PageInheritance::LegacyInitialValues` selects the
/// L3 legacy exception (resolution against the initial values — see
/// [`PageInheritance`]).
///
/// Panics unless the winner is an already-resolved `Absolute` — a relative
/// keyword surviving into the public map is exactly the regression
/// documented above.
fn page_font_weight(decl: &str, inheritance: PageInheritance<'_>) -> f32 {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&format!("@page {{ font-weight: {decl} }}"), Origin::Author);
    let result = cascade_page(&tree, &PageContextQuery::default(), inheritance);
    match result.declarations().get(&PropertyKey::FontWeight) {
        Some(PropertyValue::FontWeight(FontWeightValue::Absolute(w))) => *w,
        Some(other) => panic!(
            "regression: an unresolved font-weight value \
                 reached the public PageCascadeResult.declarations — expected \
                 FontWeight(Absolute(_)), got {other:?}"
        ),
        None => panic!(
            "no font-weight winner in PageCascadeResult.declarations for \
                 `@page {{ font-weight: {decl} }}` — the declaration failed to \
                 parse or the cascade dropped it (this is *not* the \
                 relative-font-weight resolution path)"
        ),
    }
}

#[test]
fn cascade_page_font_weight_bolder_resolves_against_root_computed_weight() {
    // All six `bolder` rows of the CSS Fonts 4 §2.2.1 table, selected by the
    // *root element's* computed weight (the page context's inheritance
    // parent per CSS Page 3 §6).
    let bolder = |root_w| {
        page_font_weight(
            "bolder",
            PageInheritance::FromRoot(&root_with_weight(root_w)),
        )
    };
    assert_eq!(bolder(50.0), 400.0, "w < 100 row");
    assert_eq!(bolder(100.0), 400.0, "100 <= w < 350 row");
    assert_eq!(bolder(400.0), 700.0, "350 <= w < 550 row");
    assert_eq!(bolder(700.0), 900.0, "550 <= w < 750 row");
    assert_eq!(bolder(800.0), 900.0, "750 <= w < 900 row");
    assert_eq!(
        bolder(1000.0),
        1000.0,
        "900 <= w row is no-change: must stay 1000, not clamp to 900"
    );
}

#[test]
fn cascade_page_font_weight_lighter_resolves_against_root_computed_weight() {
    // Same six rows, `lighter` column.
    let lighter = |root_w| {
        page_font_weight(
            "lighter",
            PageInheritance::FromRoot(&root_with_weight(root_w)),
        )
    };
    assert_eq!(
        lighter(50.0),
        50.0,
        "w < 100 row is no-change: must stay 50, not rise to 100"
    );
    assert_eq!(lighter(100.0), 100.0, "100 <= w < 350 row");
    assert_eq!(lighter(400.0), 100.0, "350 <= w < 550 row");
    assert_eq!(lighter(700.0), 400.0, "550 <= w < 750 row");
    assert_eq!(lighter(800.0), 700.0, "750 <= w < 900 row");
    assert_eq!(lighter(1000.0), 700.0, "900 <= w row");
}

// ── font-size in the page context ───────────────────────────────────────
//
// CSS Page 3 §6 "Page Properties"
// <https://www.w3.org/TR/css-page-3/#page-properties> verbatim: "When used
// on the font-size property in the page context, they are relative to the
// font-size of the root element." `resolve_against_inherited` resolves it
// so the public `declarations` map carries an absolute `Length::Px`.

/// Cascade `@page { font-size: <decl> }` and return the resolved px value.
/// `PageInheritance::LegacyInitialValues` selects the L3 legacy exception
/// (resolution against the initial values — see [`PageInheritance`]).
///
/// Panics unless the winner is an already-absolutized `Length::Px` — a
/// font-relative unit surviving into the public map is the same class of
/// regression documented above for `FontWeightValue::Bolder`.
fn page_font_size_px(decl: &str, inheritance: PageInheritance<'_>) -> f32 {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&format!("@page {{ font-size: {decl} }}"), Origin::Author);
    let result = cascade_page(&tree, &PageContextQuery::default(), inheritance);
    match result.declarations().get(&PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(Length::Px(v))) => *v,
        Some(other) => panic!(
            "an unresolved font-size value reached the public \
                 PageCascadeResult.declarations — expected FontSize(Px(_)), \
                 got {other:?}"
        ),
        None => panic!(
            "no font-size winner in PageCascadeResult.declarations for \
                 `@page {{ font-size: {decl} }}`"
        ),
    }
}

/// Root [`ComputedValues`] with `font-size: px`, everything else initial.
fn root_with_font_size(px: f32) -> ComputedValues {
    ComputedValues {
        font_size: ComputedLength(px),
        ..ComputedValues::initial()
    }
}

#[test]
fn cascade_page_font_size_resolves_against_root_computed_font_size() {
    let root = root_with_font_size(20.0);
    // §6: em on font-size in the page context is relative to the root
    // element's font-size.
    assert_eq!(
        page_font_size_px("2em", PageInheritance::FromRoot(&root)),
        40.0
    );
    // CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>: rem is
    // the computed font-size of the root element — the same basis here.
    assert_eq!(
        page_font_size_px("1.5rem", PageInheritance::FromRoot(&root)),
        30.0
    );
    // Derived (not stated by §6): CSS Fonts 4 §2.5 "Percentages: refer to
    // parent element's font size" + §6 "The page context inherits from the
    // root element."
    assert_eq!(
        page_font_size_px("150%", PageInheritance::FromRoot(&root)),
        30.0
    );
    // Absolute units are context-independent.
    assert_eq!(
        page_font_size_px("18px", PageInheritance::FromRoot(&root)),
        18.0
    );
    assert_eq!(
        page_font_size_px("12pt", PageInheritance::FromRoot(&root)),
        16.0
    );
}

#[test]
fn cascade_page_font_size_with_legacy_initial_values_uses_initial_16px() {
    // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
    // (initial values). §6 also
    // records the matching conformance exception for em/ex on font-size:
    // "an implementation that treats em and ex on font-size as relative to
    // the initial value is also conformant".
    assert_eq!(
        page_font_size_px("2em", PageInheritance::LegacyInitialValues),
        32.0
    );
    assert_eq!(
        page_font_size_px("2rem", PageInheritance::LegacyInitialValues),
        32.0
    );
}

/// `<relative-size>` (`larger` / `smaller`) in the page
/// context resolves against the root's computed font-size, same basis as
/// `em` / `rem` above (CSS Page 3 §6). Reuses `page_font_size_px`, which
/// already panics on an unresolved winner reaching `declarations` — a
/// `FontSizeRelative` leak here is the same regression class documented
/// above.
#[test]
fn cascade_page_font_size_relative_resolves_against_root_computed_font_size() {
    let root = root_with_font_size(20.0);
    assert_eq!(
        page_font_size_px("larger", PageInheritance::FromRoot(&root)),
        24.0
    );
    assert_eq!(
        page_font_size_px("smaller", PageInheritance::FromRoot(&root)),
        20.0 / 1.2
    );
}

#[test]
fn cascade_page_font_size_relative_with_legacy_initial_values_uses_initial_16px() {
    // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
    // (initial values, 16px).
    assert_eq!(
        page_font_size_px("larger", PageInheritance::LegacyInitialValues),
        19.2
    );
}

// ── declarations may carry non-finite f32 ───────────────────────────────
//
// This is not guarded here — see the `# Non-finite values pass through
// unguarded` section of
// `PageCascadeResult::declarations`'s doc for why that is intentional
// (the sink-boundary precedent described there) rather than an
// oversight. This test exists to check that the hazard is *real*, so a
// future reader cannot dismiss the doc's claim as theoretical, and so a
// regression that added a clamp here (which would violate the
// precedent) has to delete this test rather than merely adjust it.

#[test]
fn cascade_page_font_size_can_carry_nan_from_pathological_em() {
    // Same overflow-then-multiply mechanism as the element path's
    // reproducer (`crates/raikiri-dom/src/layout.rs`, "Reproducer A'"):
    // `1e40` overflows `f32` to `+Inf` at parse time (cssparser's f64 →
    // f32 conversion), and phase 2's `resolve_font_size`
    // (`parent_font_size.0 * v`) computes `0.0 * inf` = `NaN` per IEEE
    // 754 — root font-size 0 supplies the `0.0`.
    let root = root_with_font_size(0.0);
    let px = page_font_size_px("1e40em", PageInheritance::FromRoot(&root));
    // cov:ignore: the panic-message literal below is only executed if
    // the assertion fails, which it doesn't while this test passes.
    assert!(
        px.is_nan(),
        "expected NaN from `0.0 * inf` (root font-size 0 times an overflowed `1e40em`), got {px} — either the overflow/multiply mechanism changed (update this test and the `declarations` doc together) or a guard was added in raikiri-style (which would violate the sink-boundary precedent — see the doc's rationale before doing that)"
    );
}

#[test]
fn cascade_page_font_size_can_carry_infinity_from_finite_operand_multiply() {
    // Arithmetic-only counterpart to the test above: both operands are
    // already finite `f32` values (no contested f64→f32 cast involved,
    // unlike the `1e40em` reproducer — see the open question noted
    // above). `1e30` (root font-size) and `1e20` (page
    // em multiplier) are each well within f32's finite range on their
    // own; their product, `1e50`, overflows f32 (max ~3.4e38) to
    // `+Infinity` per IEEE 754. This pins that the non-finite hazard
    // documented on `PageCascadeResult::declarations` holds
    // independently of how that parse-time question is resolved.
    let root = root_with_font_size(1e30);
    let px = page_font_size_px("1e20em", PageInheritance::FromRoot(&root));
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        px.is_infinite() && px.is_sign_positive(),
        "expected +Infinity from `1e30 * 1e20` overflowing f32, got {px}"
    );
}

// ── page context inheritance: relative font-weight resolution (cont'd) ──

#[test]
fn cascade_page_font_weight_relative_with_legacy_initial_values_uses_initial_400() {
    // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
    // quoted above ("sets inherited properties in the page context to
    // their initial values").
    // `font-weight` initial is 400, so bolder(400) = 700 and
    // lighter(400) = 100.
    assert_eq!(
        page_font_weight("bolder", PageInheritance::LegacyInitialValues),
        700.0
    );
    assert_eq!(
        page_font_weight("lighter", PageInheritance::LegacyInitialValues),
        100.0
    );
}

#[test]
fn cascade_page_font_weight_absolute_ignores_root_computed_weight() {
    // Absolute weights are not inherited-value dependent: the root weight
    // must not perturb them (round-trip through `resolve_against_inherited`
    // is lossless).
    let root = root_with_weight(900.0);
    assert_eq!(
        page_font_weight("250", PageInheritance::FromRoot(&root)),
        250.0
    );
    assert_eq!(
        page_font_weight("bold", PageInheritance::FromRoot(&root)),
        700.0
    );
    assert_eq!(
        page_font_weight("normal", PageInheritance::FromRoot(&root)),
        400.0
    );
}

#[test]
fn cascade_page_resolution_runs_on_the_cascade_winner_only() {
    // The winner is picked *first*, then resolved once. A losing `bolder`
    // must not contribute, and a winning `bolder` must resolve against the
    // root weight — not against the weight declared by the losing rule
    // (the page context inherits from the root element, never from another
    // `@page` declaration).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { font-weight: 100 } @page { font-weight: bolder }",
        Origin::Author,
    );
    let root = root_with_weight(700.0);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::FontWeight),
        Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
        "later `bolder` wins and resolves off root 700 → 900, not off the \
             losing declaration's 100 → 400"
    );
}

#[test]
fn cascade_page_non_font_weight_winners_pass_through_resolution_unchanged() {
    // The pass-through arm of `resolve_against_inherited` must not perturb
    // properties that carry no inherited-value dependency, even when the
    // root style differs from the declared value.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { color: red }", Origin::Author);
    let root = ComputedValues {
        color: BLUE,
        ..ComputedValues::initial()
    };
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    assert_eq!(color_of(&result), Some(RED));
}

#[test]
fn cascade_page_length_em_is_absolutized_against_page_context_font_size() {
    // Was `cascade_page_length_em_passes_through_as_specified_value` until
    // phase 3 was added. CSS Paged Media 3
    // §6 <https://www.w3.org/TR/css-page-3/#page-properties>: "Values in
    // units of em and ex are interpreted relative to the font associated
    // with their context". With no `font-size` in the page context the
    // associated font is the inherited one ("The page context inherits from
    // the root element"), i.e. the initial 16px here → 32px.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { margin-top: 2em }", Origin::Author);
    let root = root_with_weight(700.0);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            32.0
        )))),
        "`em` must be resolved against the page context's font-size"
    );
}

#[test]
fn cascade_page_resolves_var_margin_without_raw_custom_or_deferred_values() {
    // CSS Paged Media 3 §6 gives the page context a computed value for
    // every property. The public declaration bag therefore must not leak
    // the specified-layer `Deferred` or the synthetic `Custom` key.
    let root = ComputedValues::initial();
    let result = page(
        "@page { --page-margin: 2px; margin: var(--page-margin, 1px) }",
        &root,
    );
    let expected = PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(2.0)));
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&expected)
    );
    assert!(!result.declarations().contains_key(&PropertyKey::Custom));
    assert!(result.declarations().values().all(|value| {
        !matches!(
            value,
            PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_)
        )
    }));
}

#[test]
fn cascade_page_resolves_var_outline_shorthand_into_all_longhands() {
    // The page parser expands `outline: var(--outline)` before winner
    // selection, so page-side deferred resolution must project the
    // reparsed `Outline` value for each selected longhand.
    let result = page(
        "@page { --outline: auto 2px red; outline: var(--outline) }",
        &ComputedValues::initial(),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(2.0)))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineStyle),
        Some(&PropertyValue::OutlineStyle(OutlineStyle::Auto))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineColor),
        Some(&PropertyValue::OutlineColor(OutlineColor::Resolved(RED)))
    );
    assert!(!result.declarations().contains_key(&PropertyKey::Custom));
    assert!(result.declarations().values().all(|value| {
        !matches!(
            value,
            PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_)
        )
    }));
}

#[test]
fn cascade_page_resolves_custom_property_inherited_from_root() {
    // CSS Variables 1 marks custom properties as inherited, and CSS Paged
    // Media 3 §6 makes the root element the page context's inheritance
    // parent. The page pass must therefore receive the root's resolved
    // custom-property map without exposing it as a `PropertyKey` entry.
    let mut doc = TestDoc::new();
    let root_id = doc.push_element(0, "html", Some("--page-margin: 3px"));
    let tree = build_rule_tree(&doc);
    let cascaded = crate::cascade::cascade(&doc, &tree).expect("cascade Ok");
    let result = page(
        "@page { margin: var(--page-margin, 1px) }",
        &cascaded.computed[root_id],
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            3.0
        ))))
    );
}

#[test]
fn cascade_page_drops_invalid_deferred_value_at_computed_time() {
    let root = ComputedValues::initial();
    let result = page("@page { width: var(--missing); color: red }", &root);
    assert!(!result.declarations().contains_key(&PropertyKey::Width));
    assert!(result.declarations().values().all(|value| {
        !matches!(
            value,
            PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_)
        )
    }));
}

// ── Phase 3 in the page context ─────────────────────────────────────────
//
// Primary sources, fetched as raw HTML (`curl -sL`) so the `data-level`
// attributes are visible:
// - CSS Paged Media 3 §6 "Page Properties"
//   <https://www.w3.org/TR/css-page-3/#page-properties>
// - CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
//   <https://www.w3.org/TR/css-backgrounds-3/#border-width>
// - CSS Values 4 §6.1.1 `rem` <https://www.w3.org/TR/css-values-4/#rem>

// `root_with_font_size` (above) supplies a non-initial root font-size so
// the assertions below cannot pass by accident off the 16px initial.

fn padding_top_of(result: &PageCascadeResult) -> Option<Length> {
    match result.declarations().get(&PropertyKey::PaddingTop) {
        Some(PropertyValue::PaddingTop(l)) => Some(*l),
        _ => None,
    }
}

fn page(css: &str, root: &ComputedValues) -> PageCascadeResult {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(root),
    )
}

/// `@page { padding: 2em }` — the `em` basis is the page context's own
/// font-size, which here comes from the inheritance parent (§6 "The page
/// context inherits from the root element"). Root 20px → 40px.
#[test]
fn cascade_page_padding_em_uses_inherited_font_size_when_page_declares_none() {
    let root = root_with_font_size(20.0);
    let result = page("@page { padding: 2em }", &root);
    assert_eq!(padding_top_of(&result), Some(Length::Px(40.0)));
}

/// The sibling-declaration case that forces phase 3 to be its own pass:
/// `font-size` is declared in the same `@page` block, so the `em` basis is
/// the page context's *own* font-size (20px), not the root's (16px).
#[test]
fn cascade_page_padding_em_uses_own_font_size_over_inherited() {
    let root = ComputedValues::initial(); // 16px
    let result = page("@page { font-size: 20px; padding: 2em }", &root);
    assert_eq!(
        padding_top_of(&result),
        Some(Length::Px(40.0)),
        "`em` must use the page context's declared font-size, not the root's"
    );
}

/// `rem` stays relative to the **root element** even when the page context
/// declares its own `font-size` — CSS Values 4 §6.1.1: "Equal to the
/// computed value of the em unit on the root element." The page context is
/// not the root element, so this is the `finalize` case, not
/// `finalize_as_root`.
///
/// `font-size: 2em` on the page context resolves against the root (§6:
/// "When used on the font-size property in the page context, they are
/// relative to the font-size of the root element") → 32px; `padding: 1rem`
/// must still be 16px, **not** 32px.
#[test]
fn cascade_page_rem_resolves_against_root_element_not_page_context() {
    let root = ComputedValues::initial(); // 16px
    let result = page("@page { font-size: 2em; padding: 1rem }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::FontSize),
        Some(&PropertyValue::FontSize(Length::Px(32.0))),
    );
    assert_eq!(
        padding_top_of(&result),
        Some(Length::Px(16.0)),
        "`rem` is the root element's font-size, not the page context's"
    );
}

// ── `lh` / `rlh` in the page context (CSS Values 4 §6.1.1) ─────────────

/// `@page { line-height: 2; padding: 1.5lh }` — `1lh` uses the page
/// context's **own** used line-height (its own font-size × the declared
/// `<number>`), the same "own basis" story as `em`/`rem` above.
#[test]
fn cascade_page_padding_lh_uses_own_page_context_line_height() {
    let root = root_with_font_size(20.0); // page context's own font-size, undeclared here
    let result = page("@page { line-height: 2; padding: 1.5lh }", &root);
    // own used line-height = 2 * 20px = 40px; 1.5lh = 60px.
    assert_eq!(padding_top_of(&result), Some(Length::Px(60.0)));
}

/// `rlh` always refers to the **root element's** `lh`, regardless of what
/// the page context itself declares for `line-height` — CSS Values 4
/// §6.1.1 `rlh` + CSS Paged Media 3 §6 "The page context inherits from
/// the root element" (the page context is not the root element, same
/// distinction `cascade_page_rem_resolves_against_root_element_not_page_context`
/// pins for `rem`).
#[test]
fn cascade_page_padding_rlh_uses_root_line_height_not_own() {
    let root = ComputedValues {
        font_size: ComputedLength(20.0),
        line_height: ComputedLineHeight::Number(3.0), // root used = 60px
        ..ComputedValues::initial()
    };
    // Page context declares its own (different) line-height — must not
    // affect `rlh`.
    let result = page("@page { line-height: 1; padding: 1rlh }", &root);
    assert_eq!(
        padding_top_of(&result),
        Some(Length::Px(60.0)),
        "rlh must use the root element's used line-height (60px), not the \
             page context's own (20px)"
    );
}

/// The common case: no font metrics available for `normal` — falls back
/// to padding's own initial value `0`, same policy as the element path
/// (`crate::resolve::resolve_length_percentage` doc). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn cascade_page_padding_lh_falls_back_to_zero_when_line_height_normal() {
    let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
    let result = page("@page { padding: 1lh }", &root);
    assert_eq!(padding_top_of(&result), Some(Length::Px(0.0)));
}

/// Regression pin, `@page` path:
/// `margin-top: 1lh` under (the initial, unresolvable) `line-height: normal`
/// must compute to `Px(0.0)` — margin's true spec initial (CSS Box 3
/// §3.1) — **not** `Auto` (`resolve_margin_length_or_auto` —
/// element-path sibling is
/// `margin_lh_falls_back_to_zero_not_auto_when_line_height_normal` in
/// `crate::cascade`). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn cascade_page_margin_lh_falls_back_to_zero_not_auto_when_line_height_normal() {
    let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
    let result = page("@page { margin-top: 1lh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0
        )))),
        "margin-top: 1lh under line-height: normal must be Px(0.0), not Auto"
    );
}

/// `@page { line-height: 1lh }` is self-referential — CSS Values 4
/// §6.1.1, spec quote canonically documented on
/// `crate::resolve::resolve_line_height`. The page context's "parent" // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
/// for this purpose is the root element (CSS Paged Media 3 §6), which
/// is exactly what `ctx.root_line_height` already carries.
#[test]
fn cascade_page_line_height_self_reference_uses_root_as_parent() {
    let root = ComputedValues {
        font_size: ComputedLength(20.0),
        line_height: ComputedLineHeight::Number(2.0), // root used = 40px
        ..ComputedValues::initial()
    };
    let result = page("@page { line-height: 1lh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::LineHeight),
        Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            40.0
        )))),
    );
}

/// `@page { line-height: 1rlh }` — unlike `1lh` above, `rlh` is **not**
/// treated as self-referential (`resolve_line_height`'s `Length::Rlh`
/// arm ignores the `self_reference_basis` argument entirely and reads
/// `ctx.root_line_height` directly). This test happens to land on the
/// same numeric answer as `cascade_page_line_height_self_reference_uses_root_as_parent`
/// because the page context's "parent" *is* the root — the point is
/// this is not a coincidence of implementation integration but `rlh`'s own
/// plain definition ("the lh unit on the root element").
#[test]
fn cascade_page_line_height_rlh_is_not_self_referential() {
    let root = ComputedValues {
        font_size: ComputedLength(20.0),
        line_height: ComputedLineHeight::Number(2.0), // root used = 40px
        ..ComputedValues::initial()
    };
    let result = page("@page { line-height: 1rlh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::LineHeight),
        Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            40.0
        )))),
    );
}

// ── `font-size: 1lh` / `1rlh` in the page context ───────────────────────
//
// §6 does not mention `lh`/`rlh` explicitly (only `em`/`ex`, verified against
// the primary source at implementation time) — this is the same kind of
// *derivation* the `%`/`rem` cases above already rely on: "the page context
// inherits from the root element" (§6) + CSS Values 4 §6.1.1's self-reference
// clause for font-* properties. It mirrors `page_context_line_height_basis`'s
// treatment of `line-height` itself in this same context (`self_reference_parent
// = ctx.root_line_height` there) — for the page context specifically, "parent"
// and "root" are the same node (`inherited`), so `lh` and `rlh` on `font-size`
// coincide here (same note as `cascade_page_line_height_rlh_is_not_self_referential`).

/// `@page { font-size: 1.5lh }` is self-referential (same clause as
/// `cascade_page_line_height_self_reference_uses_root_as_parent` above) —
/// its basis is the root element's used line-height, **not** any
/// `line-height` the same `@page` block declares (that would be the page
/// context's *own* line-height, which is irrelevant here — `font-size` and
/// `line-height` self-reference against the same "parent" independently).
#[test]
fn cascade_page_font_size_lh_self_reference_uses_root_as_parent() {
    let root = ComputedValues {
        font_size: ComputedLength(20.0),
        line_height: ComputedLineHeight::Number(2.0), // root used = 40px
        ..ComputedValues::initial()
    };
    // Page context declares a different own line-height — must not leak in.
    let result = page("@page { line-height: 1; font-size: 1.5lh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::FontSize),
        Some(&PropertyValue::FontSize(Length::Px(60.0))), // 1.5 * 40
    );
}

/// `@page { font-size: 1.5rlh }` lands on the same answer as `1lh` above —
/// not a coincidence of implementation integration, but because the page
/// context's "parent" *is* the root element (CSS Paged Media 3 §6),
/// exactly like `cascade_page_line_height_rlh_is_not_self_referential`.
#[test]
fn cascade_page_font_size_rlh_matches_lh_because_parent_is_root() {
    let root = ComputedValues {
        font_size: ComputedLength(20.0),
        line_height: ComputedLineHeight::Number(2.0), // root used = 40px
        ..ComputedValues::initial()
    };
    let result = page("@page { line-height: 1; font-size: 1.5rlh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::FontSize),
        Some(&PropertyValue::FontSize(Length::Px(60.0))), // 1.5 * 40
    );
}

/// The common case: root's `line-height: normal` (initial, no font
/// metrics) — `font-size: 1lh` falls back to `font-size`'s own spec
/// initial (`medium` = 16px), same "no real font metrics in the style
/// layer" wall as the element path
/// (`font_size_lh_falls_back_to_initial_when_parent_line_height_is_normal`
/// in `crate::cascade`). Deliberately **not** `root_with_font_size`'s // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
/// 20px — the fallback is `font-size`'s spec initial, unconditionally,
/// not whatever font-size the root happens to declare.
#[test]
fn cascade_page_font_size_lh_falls_back_to_initial_when_root_line_height_normal() {
    let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
    let result = page("@page { font-size: 1lh }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::FontSize),
        Some(&PropertyValue::FontSize(Length::Px(INITIAL_FONT_SIZE_PX))),
    );
}

/// `page_context_line_height_basis`'s undeclared-`line-height` branch
/// inherits `ComputedLineHeight::Number` from the root and multiplies
/// it by the **page context's own** font-size — ordinary CSS inheritance
/// semantics for the unitless multiplier (CSS Inline 3 §5.1), not a
/// page-context special case. Root font-size (16px, unused for this)
/// deliberately differs from the page context's declared font-size
/// (40px) so a bug that used the root's font-size instead would be
/// caught.
#[test]
fn cascade_page_padding_lh_uses_page_context_own_font_size_for_inherited_number() {
    let root = ComputedValues {
        line_height: ComputedLineHeight::Number(1.5), // inherited by the page context
        ..ComputedValues::initial()                   // root font-size stays 16px
    };
    let result = page("@page { font-size: 40px; padding: 1lh }", &root);
    assert_eq!(
        padding_top_of(&result),
        Some(Length::Px(60.0)), // 1.5 * 40 (page's own), not 1.5 * 16 (root's)
    );
}

/// `<percentage>` on `padding` is **not** absolutized — §6: "Percentage
/// values on the margin and padding properties are relative to the
/// dimensions of the containing block", i.e. a used-value input. The
/// computed value is the percentage itself (CSS Values 4 §5.5.1), same as
/// the element path (`resolve_length_percentage`).
#[test]
fn cascade_page_padding_percentage_stays_a_percentage() {
    let root = root_with_font_size(20.0);
    let result = page("@page { padding: 25%; margin-top: 10%; width: 50% }", &root);
    assert_eq!(padding_top_of(&result), Some(Length::Percent(25.0)));
    // `<length-percentage> | auto` takes a separate resolve path from the
    // `<length-percentage>` one above — both must pass the percentage
    // through.
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Length(
            Length::Percent(10.0)
        ))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::Width),
        Some(&PropertyValue::Width(LengthOrAuto::Length(
            Length::Percent(50.0)
        ))),
    );
}

/// `pt` is an absolute unit but not the canonical one — phase 3 normalises
/// it to `px` (CSS Values 4 §6.2: 1pt = 1/72in, 1px = 1/96in → 12pt = 16px).
#[test]
fn cascade_page_absolute_units_are_normalised_to_px() {
    let root = ComputedValues::initial();
    let result = page("@page { padding: 12pt }", &root);
    assert_eq!(padding_top_of(&result), Some(Length::Px(16.0)));
}

/// The border-`*`-width style-gating case. CSS Backgrounds 3 §3.3 propdef
/// verbatim: "Computed value: absolute length, snapped as a border width;
/// zero if the border style is `none` or `hidden`" — a **computed**-layer
/// requirement, so the declared `5px` must not reach `declarations`.
#[test]
fn cascade_page_border_width_is_gated_by_border_style_none() {
    let root = ComputedValues::initial();
    let result = page(
        "@page { border-top-width: 5px; border-top-style: none }",
        &root,
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
    );
}

/// `hidden` gates identically to `none` (same §3.3 clause).
#[test]
fn cascade_page_border_width_is_gated_by_border_style_hidden() {
    let root = ComputedValues::initial();
    let result = page(
        "@page { border-top-width: 5px; border-top-style: hidden }",
        &root,
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
    );
}

/// An **undeclared** `border-*-style` is its initial value `none` — §6
/// gives the page context "a computed value for every property" and
/// `border-style` is not inherited. So a lone `border-top-width` gates to
/// zero, exactly as `ComputedValues::initial().border.top.width` does on
/// the element path.
#[test]
fn cascade_page_border_width_alone_gates_to_zero_via_initial_style() {
    let root = ComputedValues::initial();
    let result = page("@page { border-top-width: 5px }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
    );
    assert_eq!(
        crate::computed::ComputedValues::initial().border.top.width,
        ComputedLength::ZERO,
        "element path agrees — the gate is one rule, not two",
    );
}

/// A visible style lets the width through, absolutized against the page
/// context's font-size (`0.5em` of 20px = 10px). Guards against a gate that
/// zeroes everything.
#[test]
fn cascade_page_border_width_with_visible_style_is_absolutized() {
    let root = ComputedValues::initial();
    let result = page(
        "@page { font-size: 20px; border-top-width: 0.5em; border-top-style: solid }",
        &root,
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(10.0))),
    );
}

/// The style gate applies independently to each border side; all four
/// sides are checked with distinct values.
///
/// **What it does and does not catch**: the gate is binary, so of the 6
/// possible side pairings this catches the 4 that straddle it
/// (gated ↔ ungated); swapping the two gated sides (top ↔ bottom) or the
/// two ungated ones is observable only through the distinct widths, which
/// is why `6px` and `8px` differ. A mix-up *within* the gated pair stays
/// invisible here — `page_context_border_styles` guards that structurally
/// by naming the side in each match arm rather than looking it up by key.
#[test]
fn cascade_page_border_width_gate_is_per_side() {
    let root = ComputedValues::initial();
    let result = page(
        "@page { border-top-width: 5px; border-top-style: none; \
             border-right-width: 6px; border-right-style: solid; \
             border-bottom-width: 7px; border-bottom-style: none; \
             border-left-width: 8px; border-left-style: solid }",
        &root,
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderRightWidth),
        Some(&PropertyValue::BorderRightWidth(Length::Px(6.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderBottomWidth),
        Some(&PropertyValue::BorderBottomWidth(Length::Px(0.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderLeftWidth),
        Some(&PropertyValue::BorderLeftWidth(Length::Px(8.0))),
    );
}

/// `line-height: <percentage>` is absolutized against the page context's
/// own font-size — CSS Inline 3 §5.1
/// <https://www.w3.org/TR/css-inline-3/#propdef-line-height> "Percentages:
/// computed relative to 1em". 150% of 20px = 30px.
#[test]
fn cascade_page_line_height_percentage_is_absolutized() {
    let root = ComputedValues::initial();
    let result = page("@page { font-size: 20px; line-height: 150% }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::LineHeight),
        Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
            30.0
        )))),
    );
}

/// `line-height: <number>` survives phase 3 as a number — the distinction
/// is load-bearing in the computed layer (§5.1: the number is inherited and
/// multiplied by each context's own font-size). `normal` likewise stays a
/// keyword (resolved at the used-value layer).
#[test]
fn cascade_page_line_height_number_and_normal_stay_keywords() {
    let root = ComputedValues::initial();
    assert_eq!(
        page("@page { line-height: 1.5 }", &root)
            .declarations()
            .get(&PropertyKey::LineHeight),
        Some(&PropertyValue::LineHeight(LineHeight::Number(1.5))),
    );
    assert_eq!(
        page("@page { line-height: normal }", &root)
            .declarations()
            .get(&PropertyKey::LineHeight),
        Some(&PropertyValue::LineHeight(LineHeight::Normal)),
    );
}

/// `margin` / `width` / `height` take `<length-percentage> | auto`; `auto`
/// is a keyword in the computed layer and must survive phase 3.
#[test]
fn cascade_page_auto_survives_phase_3() {
    let root = ComputedValues::initial();
    let result = page("@page { margin-top: auto; width: auto }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginTop),
        Some(&PropertyValue::MarginTop(LengthOrAuto::Auto)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::Width),
        Some(&PropertyValue::Width(LengthOrAuto::Auto)),
    );
}

/// Every remaining box property goes through phase 3, not just `padding` —
/// enumerated so a missing arm in `absolutize_in_page_context` cannot hide
/// behind the `padding` tests.
#[test]
fn cascade_page_all_box_properties_are_absolutized() {
    let root = ComputedValues::initial(); // 16px
    let result = page(
        "@page { padding-right: 1em; margin-bottom: 1em; width: 1em; height: 1em }",
        &root,
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::PaddingRight),
        Some(&PropertyValue::PaddingRight(Length::Px(16.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::MarginBottom),
        Some(&PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Px(16.0)
        ))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::Width),
        Some(&PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            16.0
        )))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::Height),
        Some(&PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            16.0
        )))),
    );
}

/// `absolutize_in_page_context`'s grid arms (`gtt`'s inner
/// `grid_track_breadth_from_computed` / `grid_inflexible_breadth_from_computed`
/// / `grid_track_size_from_computed` / `grid_track_list_from_computed`
/// round-trip helpers) round-trip every `<track-breadth>` keyword,
/// `minmax()`, `fit-content()`, `repeat()`, and top-level `none` —
/// `author_grid_template_columns_track_list_absolutizes_through_cascade`
/// (`cascade.rs`) only exercises a bare `<length>` track, which leaves
/// every other shape in this round trip unreached.
#[test]
fn cascade_page_grid_template_columns_round_trips_every_track_shape_through_phase_3() {
    let root = ComputedValues::initial();
    let result = page(
        "@page { \
             grid-template-columns: 50% max-content minmax(min-content, 2fr) \
             minmax(max-content, 3fr) minmax(auto, 10px) minmax(20px, max-content) \
             minmax(20%, min-content) fit-content(40%) fit-content(25px) auto \
             repeat(2, min-content); \
             grid-template-rows: none; \
             }",
        &root,
    );
    let declarations = result.declarations();
    let Some(PropertyValue::GridTemplateColumns(GridTemplateTracks::List(list))) =
        declarations.get(&PropertyKey::GridTemplateColumns)
    else {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        panic!("expected a track list");
    };
    assert_eq!(list.components.len(), 11);
    assert_eq!(
        list.components[0],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Length(
            Length::Percent(50.0)
        )))
    );
    assert_eq!(
        list.components[1],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::MaxContent))
    );
    assert_eq!(
        list.components[2],
        GridTrackListComponent::Size(GridTrackSize::MinMax(
            GridInflexibleBreadth::MinContent,
            GridTrackBreadth::Flex(2.0),
        ))
    );
    assert_eq!(
        list.components[3],
        GridTrackListComponent::Size(GridTrackSize::MinMax(
            GridInflexibleBreadth::MaxContent,
            GridTrackBreadth::Flex(3.0),
        ))
    );
    assert_eq!(
        list.components[4],
        GridTrackListComponent::Size(GridTrackSize::MinMax(
            GridInflexibleBreadth::Auto,
            GridTrackBreadth::Length(Length::Px(10.0)),
        ))
    );
    assert_eq!(
        list.components[5],
        GridTrackListComponent::Size(GridTrackSize::MinMax(
            GridInflexibleBreadth::Length(Length::Px(20.0)),
            GridTrackBreadth::MaxContent,
        ))
    );
    assert_eq!(
        list.components[6],
        GridTrackListComponent::Size(GridTrackSize::MinMax(
            GridInflexibleBreadth::Length(Length::Percent(20.0)),
            GridTrackBreadth::MinContent,
        ))
    );
    assert_eq!(
        list.components[7],
        GridTrackListComponent::Size(GridTrackSize::FitContent(Length::Percent(40.0)))
    );
    assert_eq!(
        list.components[8],
        GridTrackListComponent::Size(GridTrackSize::FitContent(Length::Px(25.0)))
    );
    assert_eq!(
        list.components[9],
        GridTrackListComponent::Size(GridTrackSize::Breadth(GridTrackBreadth::Auto))
    );
    assert_eq!(
        list.components[10],
        GridTrackListComponent::Repeat(GridTrackRepeat {
            count: GridRepeatCount::Count(2),
            line_names: vec![vec![], vec![]],
            tracks: vec![GridTrackSize::Breadth(GridTrackBreadth::MinContent)],
        })
    );
    assert_eq!(
        declarations.get(&PropertyKey::GridTemplateRows),
        Some(&PropertyValue::GridTemplateRows(GridTemplateTracks::None)),
    );
}

/// The `PageInheritance::LegacyInitialValues` path also runs phase 3,
/// against the initial values. Guards against the absolutization being
/// wired only into the `FromRoot` branch.
#[test]
fn cascade_page_phase_3_runs_on_the_legacy_initial_values_path() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { padding: 2em }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(padding_top_of(&result), Some(Length::Px(32.0)));
}

/// Phase 3 must not re-absolutize `font-size` — phase 2 already resolved it
/// against the *inheritance parent*, and a second pass would use the page
/// context's own (already-resolved) value as the basis. Root 20px, `2em`
/// → 40px; a double application would give 80px.
#[test]
fn cascade_page_font_size_is_not_absolutized_twice() {
    let root = root_with_font_size(20.0);
    let result = page("@page { font-size: 2em }", &root);
    assert_eq!(
        result.declarations().get(&PropertyKey::FontSize),
        Some(&PropertyValue::FontSize(Length::Px(40.0))),
    );
}

/// Direct unit test of the total fallback in `page_context_font_size`: with
/// no `font-size` winner the basis is the inheritance parent's computed
/// font-size (§6 "The page context inherits from the root element").
#[test]
fn page_context_font_size_falls_back_to_inherited() {
    let root = root_with_font_size(24.0);
    assert_eq!(
        page_context_font_size(&HashMap::new(), &root),
        ComputedLength(24.0),
    );
}

/// Direct unit test of the "undeclared style = initial `none`" rule in
/// `page_context_border_styles`, on all four sides.
#[test]
fn page_context_border_styles_default_to_initial_none() {
    let styles = page_context_border_styles(&HashMap::new());
    assert_eq!(styles.top, BorderStyle::None);
    assert_eq!(styles.right, BorderStyle::None);
    assert_eq!(styles.bottom, BorderStyle::None);
    assert_eq!(styles.left, BorderStyle::None);
}

#[test]
fn page_context_outline_style_defaults_to_initial_none() {
    assert_eq!(
        page_context_outline_style(&HashMap::new()),
        OutlineStyle::None
    );
}

#[test]
fn cascade_page_outline_color_preserves_invert_and_currentcolor() {
    let root = ComputedValues::initial();
    let invert = page("@page { outline-color: invert }", &root);
    assert_eq!(
        invert.declarations().get(&PropertyKey::OutlineColor),
        Some(&PropertyValue::OutlineColor(OutlineColor::Invert))
    );

    let current = page("@page { outline-color: currentcolor }", &root);
    assert_eq!(
        current.declarations().get(&PropertyKey::OutlineColor),
        Some(&PropertyValue::OutlineColor(OutlineColor::CurrentColor))
    );
}

#[test]
fn cascade_page_outline_width_is_gated_by_outline_style_winner() {
    let root = root_with_font_size(20.0);
    let undeclared = page("@page { outline-width: 2em }", &root);
    assert_eq!(
        undeclared.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(0.0)))
    );

    let none = page("@page { outline-width: 2em; outline-style: none }", &root);
    assert_eq!(
        none.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(0.0)))
    );

    let solid = page("@page { outline-width: 2em; outline-style: solid }", &root);
    assert_eq!(
        solid.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(40.0)))
    );
}

#[test]
fn page_context_overflow_pair_collects_both_axes() {
    // Sibling of `page_context_border_styles_default_to_initial_none`
    // above — covers `page_context_overflow_pair`'s `OverflowX`/
    // `OverflowY` match arms — `resolved`'s
    // `HashMap` iteration only ever yields one winner at a time, so
    // both axes need collecting into a single `OverflowXY` before the
    // CSS Overflow 3 §3.1 cross-axis coupling gate can run in phase 3
    // (this test only pins the collection step, not the gate itself —
    // see `property::tests::resolve_overflow_*` for that).
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { overflow-x: hidden; overflow-y: scroll; }",
        Origin::Author,
    );
    let query = PageContextQuery::default();
    let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    assert_eq!(
        result.declarations().get(&PropertyKey::OverflowX),
        Some(&PropertyValue::OverflowX(OverflowValue::Hidden)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OverflowY),
        Some(&PropertyValue::OverflowY(OverflowValue::Scroll)),
    );
}

/// Exercise shorthand fall-through values directly. Normal cascade entry
/// points expand these values before phase 3; the direct test also keeps
/// the phase-3 function panic-free for defensive inputs.
#[test]
fn absolutize_in_page_context_shorthand_fall_throughs() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Padding(Sides::all(Length::Em(2.0)))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Padding(Sides::all(Length::Px(40.0))),
    );
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Margin(Sides::all(
                LengthOrAuto::Length(Length::Rem(2.0))
            ))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(32.0)))),
    );
    // Each side of the `border` shorthand gates on the style it carries
    // itself, not on `border_styles` (which describes the longhands).
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Border(Sides::all(Border {
                width: Length::Em(1.0),
                style: BorderStyle::Solid,
                color: BorderColor::CurrentColor,
            }))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Border(Sides::all(Border {
            width: Length::Px(20.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor,
        })),
    );
    // `box-shadow: none` reuses the shared empty Arc without allocating.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::BoxShadow(Arc::new(Vec::new(),))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::BoxShadow(Arc::new(Vec::new())),
    );
    // `overflow` shorthand fall-through — resolves
    // against its *own* pair, ignoring the `overflow_pair` parameter
    // (which describes the longhands, sibling note to the `border` case
    // above). `y: Hidden` is "neither visible nor clip", so `x: Visible`
    // computes to `Auto` (CSS Overflow 3 §3.1); `y` itself is unaffected
    // since `x`'s specified value (`Visible`) does not trigger the gate.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Overflow(OverflowXY {
                x: OverflowValue::Visible,
                y: OverflowValue::Hidden,
            })),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Overflow(OverflowXY {
            x: OverflowValue::Auto,
            y: OverflowValue::Hidden,
        }),
    );
    // `background` shorthand fall-through — resolves its own
    // `position`/`size` fields (distinct em/rem values per axis, same
    // field-swap-detecting shape as the `border` case above), leaving
    // the other 6 fields untouched via the struct-update `..shorthand`.
    let shorthand = BackgroundShorthand {
        color: CssColor::TRANSPARENT,
        image: BackgroundImage::None,
        repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        },
        attachment: BackgroundAttachment::Scroll,
        position: CssPosition {
            horizontal: CssPositionOffset::Start(Length::Em(1.0)),
            vertical: CssPositionOffset::Start(Length::Rem(2.0)),
        },
        size: BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Em(3.0)),
            height: LengthOrAuto::Length(Length::Rem(4.0)),
        },
        clip: VisualBox::BorderBox,
        origin: VisualBox::PaddingBox,
    };
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Background(shorthand.clone())),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Background(BackgroundShorthand {
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Px(20.0)),
                vertical: CssPositionOffset::Start(Length::Px(32.0)),
            },
            size: BackgroundSize::Explicit {
                width: LengthOrAuto::Length(Length::Px(60.0)),
                height: LengthOrAuto::Length(Length::Px(64.0)),
            },
            ..shorthand
        }),
    );
    // `font` shorthand fall-through — resolves its own `size`/
    // `line-height` fields (distinct em/rem values, same
    // field-swap-detecting shape as the `background` case above),
    // leaving the other 4 fields untouched via the struct-update
    // `..shorthand`.
    let font = FontShorthand {
        style: FontStyle::Italic,
        variant: FontVariantCaps::SmallCaps,
        weight: FontWeightValue::Absolute(700.0),
        size: FontShorthandSize::Absolute(Length::Em(1.5)),
        line_height: LineHeight::Length(Length::Rem(2.0)),
        family: Arc::new(vec![Atom::from("serif")]),
    };
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Font(font.clone())),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Font(FontShorthand {
            size: FontShorthandSize::Absolute(Length::Px(30.0)),
            line_height: LineHeight::Length(Length::Px(32.0)),
            ..font.clone()
        }),
    );
    // `Relative` size converges to the same `Absolute(Px(_))` shape the
    // `FontSizeRelative` longhand arm produces (same basis).
    let relative_font = FontShorthand {
        size: FontShorthandSize::Relative(RelativeFontSize::Larger),
        ..font.clone()
    };
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Font(relative_font)),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::Font(FontShorthand {
            size: FontShorthandSize::Absolute(Length::Px(24.0)),
            line_height: LineHeight::Length(Length::Px(32.0)),
            ..font
        }),
    );
    // `text-decoration` shorthand fall-through — only `thickness` is
    // absolutized (same basis as the `TextDecorationThickness` longhand
    // arm above); line/style/color pass through untouched. `Auto`
    // thickness survives as a keyword, `Length` converges to `Px`.
    for thickness in [
        TextDecorationThickness::Auto,
        TextDecorationThickness::Length(Length::Em(1.5)),
    ] {
        let decoration = TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::CurrentColor,
            thickness,
        };
        let expected_thickness = match thickness {
            TextDecorationThickness::Auto | TextDecorationThickness::FromFont => thickness,
            TextDecorationThickness::Length(_) => TextDecorationThickness::Length(Length::Px(30.0)),
        };
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::TextDecoration(decoration)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::TextDecoration(TextDecorationShorthand {
                thickness: expected_thickness,
                ..decoration
            }),
        );
    }
    // `text-decoration-inset` fall-through — `Auto` survives as a
    // keyword (same shape as the `Auto` thickness case above).
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::TextDecorationInset(
                TextDecorationInset::Auto
            )),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::TextDecorationInset(TextDecorationInset::Auto),
    );
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::TextDecorationInset(
                TextDecorationInset::Lengths {
                    start: Length::Em(1.5),
                    end: Length::Rem(2.0),
                },
            )),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::TextDecorationInset(TextDecorationInset::Lengths {
            start: Length::Px(30.0),
            end: Length::Px(32.0),
        }),
    );
    // `flex-basis` intrinsic keywords round-trip as keywords through
    // the `fb` helper (same shape as `Content`, which the corpus pins
    // via its own sample).
    for basis in [
        FlexBasisValue::MinContent,
        FlexBasisValue::MaxContent,
        FlexBasisValue::FitContent,
    ] {
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::FlexBasis(basis)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::FlexBasis(basis),
        );
    }
    // Bare `TextDecorationThickness` longhands — `Auto`/`FromFont`
    // survive as keywords (the `Length` shape is already pinned via the
    // corpus sample + the shorthand case above).
    for thickness in [
        TextDecorationThickness::Auto,
        TextDecorationThickness::FromFont,
    ] {
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::TextDecorationThickness(
                    thickness
                )),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::TextDecorationThickness(thickness),
        );
    }
}

/// Check absolute conversion for the two-value logical box shorthands
/// defined by CSS Logical Properties and Values Level 1 §4.2/§4.4.
#[test]
fn absolutize_in_page_context_logical_shorthand_fall_throughs() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::PaddingInline(StartEnd {
                start: Length::Em(2.0),
                end: Length::Em(3.0),
            })),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(40.0),
            end: Length::Px(60.0),
        }),
    );
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::PaddingBlock(StartEnd {
                start: Length::Em(1.0),
                end: Length::Em(4.0),
            })),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::PaddingBlock(StartEnd {
            start: Length::Px(20.0),
            end: Length::Px(80.0),
        }),
    );
    // `margin-inline`/`margin-block` additionally check that `auto` survives
    // the round-trip on whichever side carries it (`margin_lpa`'s `Auto`
    // preservation, `absolutize_in_page_context`'s `margin_lpa` doc).
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::MarginInline(StartEnd {
                start: LengthOrAuto::Length(Length::Rem(2.0)),
                end: LengthOrAuto::Auto,
            })),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::MarginInline(StartEnd {
            start: LengthOrAuto::Length(Length::Px(32.0)),
            end: LengthOrAuto::Auto,
        }),
    );
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::MarginBlock(StartEnd {
                start: LengthOrAuto::Auto,
                end: LengthOrAuto::Length(Length::Rem(3.0)),
            })),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::MarginBlock(StartEnd {
            start: LengthOrAuto::Auto,
            end: LengthOrAuto::Length(Length::Px(48.0)),
        }),
    );
}

/// Direct exercise of `absolutize_in_page_context`'s `flex-basis`/
/// `row-gap`/`column-gap` handling (the local `fb`/`lpn` helpers
/// above) across every one of their match arms — the `sample_for`
/// corpus (`page_corpus`) intentionally samples these three properties
/// with a `<length>` (`Px`-shaped) value only (the "worst case" per
/// that macro's own convention), which never reaches `fb`'s
/// `Auto`/`Content`/`Percent` arms nor `lpn`'s `Normal`/`Percent` arms.
/// This test pins all of them directly, sibling to
/// `absolutize_in_page_context_shorthand_fall_throughs` above.
#[test]
fn absolutize_in_page_context_covers_flex_basis_and_gap_arms() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    for (basis, expected) in [
        (FlexBasisValue::Content, FlexBasisValue::Content),
        (FlexBasisValue::Auto, FlexBasisValue::Auto),
        (
            FlexBasisValue::Length(Length::Percent(50.0)),
            FlexBasisValue::Length(Length::Percent(50.0)),
        ),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::FlexBasis(basis)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::FlexBasis(expected),
            "flex-basis: {basis:?}",
        );
    }

    for gap in [
        LengthOrNormal::Normal,
        LengthOrNormal::Length(Length::Percent(25.0)),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::RowGap(gap)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::RowGap(gap),
            "row-gap: {gap:?}",
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::ColumnGap(gap)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::ColumnGap(gap),
            "column-gap: {gap:?}",
        );
    }
}

/// Direct exercise of `absolutize_in_page_context`'s `BackgroundSize`
/// arm for the `cover`/`contain` keywords — `page_corpus`'s
/// `BackgroundSize` worst-case sample is always the `Explicit` variant
/// (`sample_for` 参照), so this test drives the `Cover`/`Contain` arms
/// of the local `lift` helper directly (sibling of
/// `absolutize_in_page_context_covers_flex_basis_and_gap_arms` above).
#[test]
fn absolutize_in_page_context_covers_background_size_cover_and_contain_arms() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    for size in [BackgroundSize::Cover, BackgroundSize::Contain] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::BackgroundSize(size)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::BackgroundSize(size),
            "background-size: {size:?}",
        );
    }
}

/// Direct exercise of `background_size`'s and `css_position`'s local
/// `lift` helpers' `Percent` arm — `page_corpus`'s `BackgroundSize`
/// (`Em`/`Auto`) and `BackgroundPosition` (`Em`/`Rem`) worst-case
/// samples both resolve to `Px` (`resolve_length_percentage` collapses
/// font-relative units to `Px`, never `Percent`), so a percentage
/// payload is the only way to reach `lift`'s `Percent` arm in either
/// function; sibling of
/// `absolutize_in_page_context_covers_background_size_cover_and_contain_arms`
/// above.
#[test]
fn absolutize_in_page_context_covers_background_size_and_position_percent_axis_arms() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    let size = BackgroundSize::Explicit {
        width: LengthOrAuto::Length(Length::Percent(30.0)),
        height: LengthOrAuto::Length(Length::Percent(40.0)),
    };
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::BackgroundSize(size)),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::BackgroundSize(size),
    );

    let position = CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(30.0)),
        vertical: CssPositionOffset::End(Length::Percent(40.0)),
    };
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::BackgroundPosition(position)),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::BackgroundPosition(position),
    );
}

/// Direct exercise of `absolutize_in_page_context`'s `VerticalAlign`
/// arm — the bare keywords stay as-is, `<length>` absolutizes against this
/// page context's own font-size (same "worst case: `Em`" shape
/// `page_corpus`'s `VerticalAlign` sample uses).
#[test]
fn absolutize_in_page_context_covers_vertical_align_length_arm() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    for (specified, expected) in [
        (VerticalAlign::Baseline, VerticalAlign::Baseline),
        (VerticalAlign::Middle, VerticalAlign::Middle),
        (VerticalAlign::TextTop, VerticalAlign::TextTop),
        (VerticalAlign::TextBottom, VerticalAlign::TextBottom),
        (VerticalAlign::Top, VerticalAlign::Top),
        (VerticalAlign::Bottom, VerticalAlign::Bottom),
        (
            VerticalAlign::Length(Length::Em(2.0)),
            VerticalAlign::Length(Length::Px(40.0)),
        ),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::VerticalAlign(specified)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::VerticalAlign(expected),
            "vertical-align: {specified:?}",
        );
    }
}

/// Direct exercise of `absolutize_in_page_context`'s `TabSize` arm —
/// `<number>` stays as-is (CSS Text Module Level 3 §4.2: "Computed
/// value: the specified number or absolute length"), `<length>`
/// absolutizes against this page context's own font-size (same "worst
/// case: `Em`" shape `page_corpus`'s `TabSize` sample uses).
#[test]
fn absolutize_in_page_context_covers_tab_size_arm() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    for (specified, expected) in [
        (TabSize::Number(4.0), TabSize::Number(4.0)),
        (
            TabSize::Length(Length::Em(2.0)),
            TabSize::Length(Length::Px(40.0)),
        ),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::TabSize(specified)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::TabSize(expected),
            "tab-size: {specified:?}",
        );
    }
}

/// CSS Writing Modes resolution currently maps all supported writing
/// modes to the horizontal-tb computed value.
#[test]
fn absolutize_in_page_context_collapses_writing_mode_to_horizontal_tb() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    for specified in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::WritingMode(specified)),
                fs,
                None,
                &ctx,
                styles,
                OutlineStyle::None,
                OverflowXY::both(OverflowValue::Visible),
            ),
            PropertyValue::WritingMode(WritingMode::HorizontalTb),
            "writing-mode: {specified:?}",
        );
    }
}

/// Direct exercise of the `TextShadow` arm of
/// `absolutize_in_page_context` — both the `none` (empty-list) fast
/// path, which reuses the input `Arc` without allocating, and the
/// non-empty path, which absolutizes each item's `offset_x`/`offset_y`/
/// `blur_radius` against this context's own font-size basis (CSS Text
/// Decoration Module Level 3 §4).
#[test]
fn absolutize_in_page_context_covers_text_shadow_arm() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::TextShadow(Arc::new(Vec::new()))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::TextShadow(Arc::new(Vec::new())),
        "text-shadow: none",
    );

    let specified = TextShadowItem {
        offset_x: Length::Em(1.0),
        offset_y: Length::Em(2.0),
        blur_radius: Length::Em(0.5),
        color: TextShadowColor::CurrentColor,
    };
    let expected = TextShadowItem {
        offset_x: Length::Px(20.0),
        offset_y: Length::Px(40.0),
        blur_radius: Length::Px(10.0),
        color: TextShadowColor::CurrentColor,
    };
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::TextShadow(Arc::new(vec![
                specified
            ]))),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::TextShadow(Arc::new(vec![expected])),
        "text-shadow: 1em 2em 0.5em currentcolor",
    );
}

/// CSS Color 4 §3.3 clamps computed opacity to `[0, 1]`.
#[test]
fn absolutize_in_page_context_covers_opacity_arm_clamp() {
    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);
    let outline = OutlineStyle::None;
    let overflow = OverflowXY::both(OverflowValue::Visible);

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Opacity(2.0)),
            fs,
            None,
            &ctx,
            styles,
            outline,
            overflow,
        ),
        PropertyValue::Opacity(1.0),
        "opacity: 2 clamps to 1.0",
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Opacity(-0.5)),
            fs,
            None,
            &ctx,
            styles,
            outline,
            overflow,
        ),
        PropertyValue::Opacity(0.0),
        "opacity: -0.5 clamps to 0.0",
    );
    // In-range value passes through unchanged.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::Opacity(0.5)),
            fs,
            None,
            &ctx,
            styles,
            outline,
            overflow,
        ),
        PropertyValue::Opacity(0.5),
        "opacity: 0.5 is already in range",
    );
}

#[test]
fn cascade_page_computes_border_radius_box_shadow_and_outline() {
    let root = root_with_font_size(20.0);
    let result = page(
        "@page { border-radius: 10% 2em 3em 4em; box-shadow: red 0.5em -1em 0.25em 0.125em, 2px 3px; outline: solid 2em red }",
        &root,
    );

    assert_eq!(
        result.declarations().get(&PropertyKey::BorderRadius),
        Some(&PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Percent(10.0),
            top_right: Length::Px(40.0),
            bottom_right: Length::Px(60.0),
            bottom_left: Length::Px(80.0),
        }))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BoxShadow),
        Some(&PropertyValue::BoxShadow(Arc::new(vec![
            BoxShadowItem {
                offset_x: Length::Px(10.0),
                offset_y: Length::Px(-20.0),
                blur_radius: Length::Px(5.0),
                spread_radius: Length::Px(2.5),
                color: TextShadowColor::Resolved(RED),
                inset: false,
            },
            BoxShadowItem {
                offset_x: Length::Px(2.0),
                offset_y: Length::Px(3.0),
                blur_radius: Length::Px(0.0),
                spread_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
                inset: false,
            },
        ])))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(40.0)))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineStyle),
        Some(&PropertyValue::OutlineStyle(OutlineStyle::Solid))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineColor),
        Some(&PropertyValue::OutlineColor(OutlineColor::Resolved(RED)))
    );
}

/// CSS Basic User Interface Module Level 3 §4: page-context outline
/// declarations use the dedicated `OutlineStyle` carrier, including the
/// outline-only `auto` keyword.
#[test]
fn cascade_page_outline_auto_survives_longhand_expansion() {
    let root = root_with_font_size(20.0);
    let result = page("@page { outline: auto 2em red }", &root);

    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineWidth),
        Some(&PropertyValue::OutlineWidth(Length::Px(40.0)))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineStyle),
        Some(&PropertyValue::OutlineStyle(OutlineStyle::Auto))
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OutlineColor),
        Some(&PropertyValue::OutlineColor(OutlineColor::Resolved(RED)))
    );
}

/// Direct phase-3 inputs may still contain a relative font size; normalize
/// them without panicking and preserve the computed value shape.
#[test]
fn absolutize_in_page_context_font_size_relative_safety_net() {
    use crate::property::RelativeFontSize;

    let fs = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    let styles = Sides::all(BorderStyle::None);

    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::FontSizeRelative(
                RelativeFontSize::Larger
            )),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::FontSize(Length::Px(24.0)),
    );
    assert_eq!(
        absolutize_in_page_context(
            ResolvedAgainstInherited::for_test(PropertyValue::FontSizeRelative(
                RelativeFontSize::Smaller
            )),
            fs,
            None,
            &ctx,
            styles,
            OutlineStyle::None,
            OverflowXY::both(OverflowValue::Visible),
        ),
        PropertyValue::FontSize(Length::Px(20.0 / 1.2)),
    );
}

// The corpus below exercises the computed-value contract and uses an
// exhaustive match so newly added property variants require an explicit
// classification.

/// Number of corpus variants that require no page-context length
/// conversion. The exhaustive match in `absolutize_in_page_context`
/// determines the classification.
// Includes page-only inherit markers, which are resolved before this
// phase and therefore remain unchanged here.
const PHASE_3_PASS_THROUGH_VARIANTS: usize = 121;
/// Number of corpus variants transformed by page-context resolution.
/// This is derived from the corpus size and the pass-through count.
fn phase_3_transformed_variants() -> usize {
    page_corpus().len() - PHASE_3_PASS_THROUGH_VARIANTS
}

/// Number of raw corpus variants that retain specified-layer values after
/// accounting for phase-2 resolution and keyword-only phase-3 transforms.
/// The count is used to check that page declarations expose computed
/// values rather than unresolved specified values.
const KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE: usize = 5;

fn raw_corpus_residue_variants() -> usize {
    // The five raw page-only inherit markers add specified-layer residue
    // just like the existing four phase-2-only samples.
    phase_3_transformed_variants() + 9 - KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE
}

/// `sample_for` / `ALL_PROPERTY_KEYS` を **1 つの token 列**から生成する。
/// `key => value` の対を 1 度書けば
/// `ALL_PROPERTY_KEYS` (列挙) と `sample_for` (網羅 match) の**両方**に
/// そのまま展開される。
///
/// 本 macro が何を置き換え、何を塞ぎ何を塞がないかの canonical な記述は
/// `page_corpus` 手前の section comment (「`declarations` は computed 値」
/// 契約の機械的 check 節) にある — 繰り返さない。
macro_rules! property_key_samples {
        ($($key:ident => $value:expr),+ $(,)?) => {
            /// `PropertyValue::key()` を経由して 1:1 対応する `PropertyKey`
            /// 全件、`property_key_samples!` 呼び出しでの記述順
            /// (= `PropertyKey` 自身の宣言順、`property.rs`)。key を複数
            /// `PropertyValue` variant で共有するもの (`FontSize` /
            /// `FontSizeRelative`) はここには 1 度しか現れない —
            /// 共有側は `key_sharing_extras` が別途持つ。
            const ALL_PROPERTY_KEYS: &[PropertyKey] = &[$(PropertyKey::$key),+];

            /// 与えられた `PropertyKey` に対する **specified 層の worst
            /// case** `PropertyValue` サンプルを 1 つ返す。
            ///
            /// **wildcard arm を置かない** (`property_key_samples!` の
            /// 展開そのものが持たない) — `PropertyKey` に variant を足すと
            /// ここで **compile error** になる。この compile error が何を
            /// 強制し (通常の新 property 追加)、何を強制しないか
            /// (`FontSizeRelative` 型の key 共有 variant) の canonical な
            /// 記述は `page_corpus` 手前の
            /// section comment にある。
            fn sample_for(key: PropertyKey) -> PropertyValue {
                match key {
                    $(PropertyKey::$key => $value,)+
                }
            }
        };
    }

property_key_samples! {
    Color => PropertyValue::Color(RED),
    BackgroundColor => PropertyValue::BackgroundColor(BLUE),
    FontFamily => PropertyValue::FontFamily(Arc::new(vec![Atom::from("serif")])),
    FontSize => PropertyValue::FontSize(Length::Em(2.0)),
    FontWeight => PropertyValue::FontWeight(FontWeightValue::Bolder),
    LineHeight => PropertyValue::LineHeight(LineHeight::Length(Length::Em(2.0))),
    Display => PropertyValue::Display(DisplayValue::Block),
    Grid => PropertyValue::Grid(GridShorthand {
        rows: GridTemplateTracks::None,
        columns: GridTemplateTracks::None,
    }),
    GridArea => PropertyValue::GridArea(GridAreaShorthand {
        row_start: GridLineValue::Line(1),
        column_start: GridLineValue::Line(1),
        row_end: GridLineValue::Line(2),
        column_end: GridLineValue::Line(2),
    }),
    CounterReset => PropertyValue::CounterReset(Arc::new(vec![("c".into(), 0)])),
    CounterIncrement => PropertyValue::CounterIncrement(Arc::new(vec![("c".into(), 1)])),
    CounterSet => PropertyValue::CounterSet(Arc::new(vec![("c".into(), 2)])),
    Content =>
        PropertyValue::Content(Arc::new(vec![ContentComponent::Literal("x".into())])),
    StringSet => PropertyValue::StringSet(Arc::new(vec![(
        "s".into(),
        vec![ContentComponent::Literal("x".into())],
    )])),
    Position => PropertyValue::Position(PositionValue::Static),
    TextAlign => PropertyValue::TextAlign(TextAlign::MatchParent),
    HangingPunctuation => PropertyValue::HangingPunctuation(HangingPunctuation::First),
    TextIndent => PropertyValue::TextIndent(TextIndentValue {
        length: Length::Em(2.0),
        hanging: false,
        each_line: false,
    }),
    PaddingTop => PropertyValue::PaddingTop(Length::Em(2.0)),
    PaddingRight => PropertyValue::PaddingRight(Length::Em(2.0)),
    PaddingBottom => PropertyValue::PaddingBottom(Length::Em(2.0)),
    PaddingLeft => PropertyValue::PaddingLeft(Length::Em(2.0)),
    Padding => PropertyValue::Padding(Sides::all(Length::Em(2.0))),
    // `PaddingInline`/`PaddingBlock` — same worst-case unit (`Em`) as
    // `Padding` above, placed right after it to match `PropertyKey`'s
    // own declaration order (`property.rs`'s "shorthand key comes after
    // the longhands it can compete with" placement, `PropertyKey` doc's
    // "宣言順は load-bearing" section).
    PaddingInline => PropertyValue::PaddingInline(StartEnd::both(Length::Em(2.0))),
    PaddingBlock => PropertyValue::PaddingBlock(StartEnd::both(Length::Em(2.0))),
    MarginTop => PropertyValue::MarginTop(LengthOrAuto::Length(Length::Rem(2.0))),
    MarginRight => PropertyValue::MarginRight(LengthOrAuto::Length(Length::Rem(2.0))),
    MarginBottom => PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Rem(2.0))),
    MarginLeft => PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Rem(2.0))),
    Margin => PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Rem(2.0)))),
    // `MarginInline`/`MarginBlock` — same placement rationale as
    // `PaddingInline`/`PaddingBlock` above.
    MarginInline =>
        PropertyValue::MarginInline(StartEnd::both(LengthOrAuto::Length(Length::Rem(2.0)))),
    MarginBlock =>
        PropertyValue::MarginBlock(StartEnd::both(LengthOrAuto::Length(Length::Rem(2.0)))),
    BorderTopWidth => PropertyValue::BorderTopWidth(Length::Pt(12.0)),
    BorderRightWidth => PropertyValue::BorderRightWidth(Length::Pt(12.0)),
    BorderBottomWidth => PropertyValue::BorderBottomWidth(Length::Pt(12.0)),
    BorderLeftWidth => PropertyValue::BorderLeftWidth(Length::Pt(12.0)),
    BorderTopStyle => PropertyValue::BorderTopStyle(BorderStyle::Solid),
    BorderRightStyle => PropertyValue::BorderRightStyle(BorderStyle::Solid),
    BorderBottomStyle => PropertyValue::BorderBottomStyle(BorderStyle::Solid),
    BorderLeftStyle => PropertyValue::BorderLeftStyle(BorderStyle::Solid),
    BorderTopColor => PropertyValue::BorderTopColor(BorderColor::CurrentColor),
    BorderRightColor => PropertyValue::BorderRightColor(BorderColor::CurrentColor),
    BorderBottomColor => PropertyValue::BorderBottomColor(BorderColor::CurrentColor),
    BorderLeftColor => PropertyValue::BorderLeftColor(BorderColor::CurrentColor),
    Border => PropertyValue::Border(Sides::all(Border {
        width: Length::Em(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    })),
    BorderStyle => PropertyValue::BorderStyle(Sides::all(BorderStyle::Double)),
    BorderWidth => PropertyValue::BorderWidth(Sides::all(Length::Em(2.0))),
    BorderColor => PropertyValue::BorderColor(Sides::all(BorderColor::CurrentColor)),
    Width => PropertyValue::Width(LengthOrAuto::Length(Length::Em(3.0))),
    Height => PropertyValue::Height(LengthOrAuto::Length(Length::Em(4.0))),
    MaxWidth => PropertyValue::MaxWidth(LengthOrAuto::Length(Length::Em(5.0))),
    MaxHeight => PropertyValue::MaxHeight(LengthOrAuto::Length(Length::Em(6.0))),
    MinWidth => PropertyValue::MinWidth(LengthOrAuto::Length(Length::Em(7.0))),
    MinHeight => PropertyValue::MinHeight(LengthOrAuto::Length(Length::Em(8.0))),
    MinBlockSize => PropertyValue::MinBlockSize(LengthOrAuto::Length(Length::Em(9.0))),
    TextUnderlineOffset => {
        PropertyValue::TextUnderlineOffset(LengthOrAuto::Length(Length::Em(9.0)))
    },
    BoxSizing => PropertyValue::BoxSizing(BoxSizing::BorderBox),
    // No specified/computed distinction for `direction` (computed
    // value = specified value) — any value is "worst case".
    Direction => PropertyValue::Direction(Direction::Rtl),
    // `Visible` is deliberately the "worst case" here
    // — it is the one value the CSS Overflow 3 §3.1 cross-axis coupling
    // actually rewrites (`visible` -> `auto`) when the fixed test
    // `overflow_pair`/pair fixtures used by the corpus-driven tests below
    // pair it with a non-visible/clip other axis (see
    // `phase_3_variant_classification_matches_the_documented_counts`).
    OverflowX => PropertyValue::OverflowX(OverflowValue::Visible),
    // `Clip` is the other value the coupling rewrites (`clip` -> `hidden`).
    OverflowY => PropertyValue::OverflowY(OverflowValue::Clip),
    // Self-contained pair whose own two axes already trigger the coupling
    // (`x: Visible` paired with `y: Hidden`, a "neither visible nor clip"
    // value) — the `Overflow` shorthand fall-through arm reads only its
    // own payload, not the external `overflow_pair` parameter.
    Overflow => PropertyValue::Overflow(OverflowXY {
        x: OverflowValue::Visible,
        y: OverflowValue::Hidden,
    }),
    // No specified/computed distinction for `text-decoration-line`/
    // `-style`/`-color` (computed value = specified keyword(s)/color,
    // `TextDecorationLine`/`TextDecorationStyle`/`TextDecorationColor`
    // docs) — any value is "worst case" (`Direction` sibling comment
    // above uses the same reasoning).
    TextDecorationLine => PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE),
    TextDecorationStyle => PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy),
    TextDecorationColor =>
        PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor),
    TextDecoration => PropertyValue::TextDecoration(TextDecorationShorthand {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Wavy,
        color: TextDecorationColor::CurrentColor,
        thickness: TextDecorationThickness::Length(Length::Em(0.5)),
    }),
    // CSS Text Decoration 4 ED §2.10.4 — keyword-only, carries no
    // length. `All` is the non-initial worst case (`auto` is the spec
    // initial, same reasoning as `BackgroundAttachment`'s `Fixed`
    // sample above).
    TextDecorationSkipInk => {
        PropertyValue::TextDecorationSkipInk(TextDecorationSkipInk::All)
    },
    // ED §2.10.3 — keyword-only, carries no length. `All` is distinct
    // from the spec initial (`start end`, not `all`).
    TextDecorationSkipSpaces => {
        PropertyValue::TextDecorationSkipSpaces(TextDecorationSkipSpaces::All)
    },
    // ED §2.4.1 — `Length::Em` worst-case payload (same convention as
    // `BackgroundSize` above) so this raw sample is caught by
    // `specified_layer_residue` before phase 3 absolutizes it.
    TextDecorationThickness => PropertyValue::TextDecorationThickness(
        TextDecorationThickness::Length(Length::Em(0.5)),
    ),
    // ED §2.9.1 — `Em`/`Rem` worst-case payloads on start/end
    // (distinct edges, same convention as `BackgroundPosition` above).
    TextDecorationInset => {
        PropertyValue::TextDecorationInset(TextDecorationInset::Lengths {
            start: Length::Em(1.0),
            end: Length::Rem(2.0),
        })
    },
    // ED §3.4 — keyword-only, carries no length. Both axes non-initial
    // (`over right` is the spec initial).
    TextEmphasisPosition => PropertyValue::TextEmphasisPosition(
        TextEmphasisPosition::Position {
            vertical: TextEmphasisVEdge::Under,
            horizontal: Some(TextEmphasisHEdge::Left),
        },
    ),
    // ED §2.7 — keyword-only, carries no length. Non-`auto` worst case.
    TextUnderlinePosition => PropertyValue::TextUnderlinePosition(TextUnderlinePosition {
        from_font: false,
        under: true,
        left: false,
        right: true,
    }),
    // Unlike `Direction`/`TextDecorationLine` above, `vertical-align`
    // *does* have a specified/computed distinction now that it carries
    // a `<length>` variant — `Em` (not `Px`) is the worst-case payload,
    // same "exercise phase-3 absolutization" reasoning as
    // `LetterSpacing`/`FlexBasis` below.
    VerticalAlign => PropertyValue::VerticalAlign(VerticalAlign::Length(Length::Em(0.3))),
    // No specified/computed distinction for `font-style` at this
    // crate's scope (computed value = specified keyword, `FontStyle`
    // doc) — any value is "worst case" (`Direction` sibling comment
    // above uses the same reasoning).
    FontStyle => PropertyValue::FontStyle(FontStyle::Italic),
    // No specified/computed distinction for `text-transform`
    // (computed value = specified keyword, `TextTransform` doc) — any
    // value is "worst case" (`Direction` sibling comment above uses
    // the same reasoning).
    TextTransform => PropertyValue::TextTransform(TextTransform::Uppercase),
    // No specified/computed distinction for `visibility` (computed
    // value = specified keyword, `Visibility` doc) — any value is
    // "worst case" (`Direction` sibling comment above uses the same
    // reasoning). `Collapse` chosen over `Hidden`/`Visible` since it is
    // the keyword whose formatting-context-specific behavior this crate
    // does not implement (`Visibility` doc's "Scope carving" section).
    Visibility => PropertyValue::Visibility(Visibility::Collapse),
    // No specified/computed distinction for `z-index` (computed value =
    // specified value, `ZIndexValue` doc) — any value is "worst case"
    // (`Direction` sibling comment above uses the same reasoning). A
    // negative integer exercises the non-`Auto` branch without being
    // mistakable for the zero the `auto` keyword computes to.
    ZIndex => PropertyValue::ZIndex(ZIndexValue::Integer(-3)),
    // No specified/computed distinction for `word-break` (computed
    // value = specified keyword, `WordBreak` doc) — any value is
    // "worst case" (`Direction` sibling comment above uses the same
    // reasoning).
    WordBreak => PropertyValue::WordBreak(WordBreak::BreakAll),
    // No specified/computed distinction for `overflow-wrap` (computed
    // value = specified keyword, `OverflowWrap` doc) — any value is
    // "worst case" (`Direction` sibling comment above uses the same
    // reasoning).
    OverflowWrap => PropertyValue::OverflowWrap(OverflowWrap::Anywhere),
    // `Em`/`Rem` (not `Px`) — same "worst case" reasoning as `Border`/
    // `Width`/`Height`/`MarginTop` above: a font-relative unit exercises
    // phase-3 absolutization (`resolve_length_or_normal`) instead of
    // trivially round-tripping an already-absolute length.
    LetterSpacing => PropertyValue::LetterSpacing(LengthOrNormal::Length(Length::Em(0.1))),
    WordSpacing => PropertyValue::WordSpacing(LengthOrNormal::Length(Length::Rem(0.2))),
    // No specified/computed distinction for `break-before`/
    // `break-after` (computed value = specified keyword, `BreakBetween`
    // doc) — any value is "worst case" (`Direction` sibling comment
    // above uses the same reasoning). `AvoidPage`/`Page` chosen over
    // `Auto` so the sample is not the initial value.
    BreakBefore => PropertyValue::BreakBefore(BreakBetween::AvoidPage),
    BreakAfter => PropertyValue::BreakAfter(BreakBetween::Page),
    // No specified/computed distinction for `break-inside` either
    // (computed value = specified keyword, `BreakInside` doc) — same
    // reasoning.
    BreakInside => PropertyValue::BreakInside(BreakInside::AvoidPage),
    // No specified/computed distinction for `float` (computed value =
    // specified value, `FloatValue` doc) — any value is "worst case"
    // (`Direction` sibling comment above uses the same reasoning).
    Float => PropertyValue::Float(FloatValue::Left),
    // No specified/computed distinction for `clear` (computed value =
    // specified value, `ClearValue` doc) — any value is "worst case"
    // (`Direction` sibling comment above uses the same reasoning).
    Clear => PropertyValue::Clear(ClearValue::Both),
    // No specified/computed distinction for `white-space` (computed
    // value = specified keyword, `WhiteSpace` doc) — any value is
    // "worst case" (`Direction` sibling comment above uses the same
    // reasoning). `Pre` chosen over `Normal` since it is a non-initial
    // keyword, the same "not the initial value" reasoning
    // `WordBreak`/`OverflowWrap` samples above use.
    WhiteSpace => PropertyValue::WhiteSpace(WhiteSpace::Pre),
    TextWrap => PropertyValue::TextWrap(TextWrapMode::Nowrap),
    // No specified/computed distinction for `flex-direction`/`flex-wrap`
    // (computed value = specified keyword) — any value is "worst case"
    // (`Direction` sibling comment above uses the same reasoning).
    FlexDirection => PropertyValue::FlexDirection(FlexDirectionValue::RowReverse),
    FlexWrap => PropertyValue::FlexWrap(FlexWrapValue::WrapReverse),
    // No specified/computed distinction for `flex-grow`/`flex-shrink`
    // (computed value = specified number, no length payload) — any
    // value is "worst case".
    FlexGrow => PropertyValue::FlexGrow(2.5),
    FlexShrink => PropertyValue::FlexShrink(0.5),
    // `Em` (not `Px`) — same "worst case" reasoning as `Width`/`Height`
    // above: a font-relative unit exercises phase-3 absolutization
    // (`resolve_flex_basis`) instead of trivially round-tripping an
    // already-absolute length.
    FlexBasis => PropertyValue::FlexBasis(FlexBasisValue::Length(Length::Em(2.0))),
    Flex => PropertyValue::Flex(FlexShorthand {
        grow: 2.0,
        shrink: 1.0,
        basis: FlexBasisValue::Length(Length::Rem(1.5)),
    }),
    FlexFlow => PropertyValue::FlexFlow(FlexFlow {
        direction: FlexDirectionValue::Column,
        wrap: FlexWrapValue::Wrap,
    }),
    Order => PropertyValue::Order(5),
    // No specified/computed distinction for `justify-content`/
    // `align-content`/`align-items`/`align-self` — any value is "worst
    // case" (`Direction` sibling comment above uses the same reasoning).
    JustifyContent => PropertyValue::JustifyContent(ContentAlignmentValue::SpaceBetween),
    AlignContent => PropertyValue::AlignContent(ContentAlignmentValue::Center),
    AlignItems => PropertyValue::AlignItems(SelfAlignmentValue::FlexEnd),
    AlignSelf => PropertyValue::AlignSelf(AlignSelfValue::Value(SelfAlignmentValue::Center)),
    // `Em`/`Rem` (not `Px`) — same "worst case" reasoning as
    // `LetterSpacing`/`WordSpacing` above.
    RowGap => PropertyValue::RowGap(LengthOrNormal::Length(Length::Em(0.5))),
    ColumnGap => PropertyValue::ColumnGap(LengthOrNormal::Length(Length::Rem(0.75))),
    Gap => PropertyValue::Gap(GapShorthand {
        row: LengthOrNormal::Length(Length::Em(0.5)),
        column: LengthOrNormal::Length(Length::Rem(0.75)),
    }),
    PlaceContent => PropertyValue::PlaceContent(PlaceContentShorthand {
        align: ContentAlignmentValue::SpaceBetween,
        justify: ContentAlignmentValue::Center,
    }),
    // No specified/computed distinction for `hyphens` (computed value =
    // specified keyword, `Hyphens` doc) — any value is "worst case"
    // (`Direction` sibling comment above uses the same reasoning).
    // `Auto` chosen over `None`/`Manual`, same "the keyword whose
    // behavior this crate does not implement" reasoning `Visibility`'s
    // `Collapse` sample above uses (`Hyphens` doc's "Downstream
    // handoff" section — dictionary-based automatic hyphenation is not
    // implemented).
    Hyphens => PropertyValue::Hyphens(Hyphens::Auto),
    // `Em` (not `Px`) — same "worst case" reasoning as `LetterSpacing`/
    // `WordSpacing` above: a font-relative unit exercises phase-3
    // absolutization (`resolve_tab_size`) instead of trivially
    // round-tripping an already-absolute length.
    TabSize => PropertyValue::TabSize(TabSize::Length(Length::Em(0.5))),
    LineBreak => PropertyValue::LineBreak(LineBreak::Auto),
    TextJustify => PropertyValue::TextJustify(TextJustify::Auto),
    TextAlignAll => PropertyValue::TextAlignAll(TextAlignAll::Start),
    TextAlignLast => PropertyValue::TextAlignLast(TextAlignLast::Auto),
    TextCombineUpright => PropertyValue::TextCombineUpright(TextCombineUpright::None),
    TextOrientation => PropertyValue::TextOrientation(TextOrientation::Mixed),
    UnicodeBidi => PropertyValue::UnicodeBidi(UnicodeBidi::Normal),
    // No specified/computed distinction for `font-variant-caps`
    // (computed value = specified keyword, `FontVariantCaps` doc) — any
    // value is "worst case" (`Direction` sibling comment above uses the
    // same reasoning). `SmallCaps` chosen over `Normal` since it is a
    // non-initial keyword, the same "not the initial value" reasoning
    // `WordBreak`/`WhiteSpace` samples above use.
    FontVariantCaps => PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps),
    // No specified/computed distinction for `quotes` (computed value =
    // specified value, `ComputedValues::quotes` doc) — any value is
    // "worst case" (`Direction` sibling comment above uses the same
    // reasoning).
    Quotes => PropertyValue::Quotes(Arc::new(vec![("«".into(), "»".into())])),
    // `Em` (not `Px`) offsets/blur — same "worst case" reasoning as
    // `FlexBasis`/`RowGap` above: exercises phase-3 absolutization
    // (`resolve_text_shadow_item`) instead of trivially round-tripping
    // an already-absolute length. `color` is `Resolved` rather than
    // `CurrentColor` so this sample also exercises the pass-through
    // (non-length) `color` field with a non-default payload.
    TextShadow => PropertyValue::TextShadow(Arc::new(vec![TextShadowItem {
        offset_x: Length::Em(0.5),
        offset_y: Length::Em(0.5),
        blur_radius: Length::Em(0.25),
        color: TextShadowColor::Resolved(GREEN),
    }])),
    // `Em`/`Rem` (not `Px`) — same "worst case" reasoning as
    // `FlexBasis`/`RowGap` above: a font-relative unit inside the track
    // list exercises phase-3 absolutization (`gtt`/`resolve_grid_template_tracks`)
    // instead of trivially round-tripping an already-absolute length.
    GridTemplateColumns => PropertyValue::GridTemplateColumns(GridTemplateTracks::List(
        Arc::new(GridTrackList {
            line_names: vec![vec![], vec![]],
            components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                GridTrackBreadth::Length(Length::Em(2.0)),
            ))],
        }),
    )),
    GridTemplateRows => PropertyValue::GridTemplateRows(GridTemplateTracks::List(Arc::new(
        GridTrackList {
            line_names: vec![vec![], vec![]],
            components: vec![GridTrackListComponent::Size(GridTrackSize::Breadth(
                GridTrackBreadth::Length(Length::Rem(1.5)),
            ))],
        },
    ))),
    // No specified/computed distinction for `grid-template-areas`
    // (computed value = specified string list, `GridTemplateAreasValue`
    // doc) — any value is "worst case".
    GridTemplateAreas => PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(
        Arc::new(GridTemplateAreas {
            row_strings: vec!["a".into()],
            areas: vec![GridTemplateAreaEntry {
                name: "a".into(),
                row_start: 1,
                row_end: 2,
                column_start: 1,
                column_end: 2,
            }],
            row_count: 1,
            column_count: 1,
        }),
    )),
    // `Em`/`Rem` (not `Px`) — same "worst case" reasoning as
    // `GridTemplateColumns`/`GridTemplateRows` above.
    GridAutoColumns => PropertyValue::GridAutoColumns(Arc::new(vec![GridTrackSize::Breadth(
        GridTrackBreadth::Length(Length::Em(1.5)),
    )])),
    GridAutoRows => PropertyValue::GridAutoRows(Arc::new(vec![GridTrackSize::Breadth(
        GridTrackBreadth::Length(Length::Rem(2.0)),
    )])),
    // No specified/computed distinction for `grid-auto-flow`/
    // `grid-row-start`/`grid-row-end`/`grid-column-start`/
    // `grid-column-end` — any value is "worst case" (`Direction`
    // sibling comment above uses the same reasoning).
    GridAutoFlow => PropertyValue::GridAutoFlow(GridAutoFlowValue::ColumnDense),
    GridRowStart => PropertyValue::GridRowStart(GridLineValue::Line(2)),
    GridRowEnd => PropertyValue::GridRowEnd(GridLineValue::Span(3)),
    GridColumnStart => PropertyValue::GridColumnStart(GridLineValue::Named("foo".into())),
    GridColumnEnd => PropertyValue::GridColumnEnd(GridLineValue::NamedLine("bar".into(), 2)),
    GridRow => PropertyValue::GridRow(GridLineShorthand {
        start: GridLineValue::Line(1),
        end: GridLineValue::Line(3),
    }),
    GridColumn => PropertyValue::GridColumn(GridLineShorthand {
        start: GridLineValue::Line(2),
        end: GridLineValue::Auto,
    }),
    JustifyItems => PropertyValue::JustifyItems(SelfAlignmentValue::Center),
    JustifySelf => PropertyValue::JustifySelf(AlignSelfValue::Value(SelfAlignmentValue::End)),
    PlaceItems => PropertyValue::PlaceItems(PlaceItemsShorthand {
        align: SelfAlignmentValue::Center,
        justify: SelfAlignmentValue::End,
    }),
    PlaceSelf => PropertyValue::PlaceSelf(PlaceSelfShorthand {
        align: AlignSelfValue::Auto,
        justify: AlignSelfValue::Value(SelfAlignmentValue::Start),
    }),
    // No specified/computed distinction for `orphans`/`widows`
    // (computed value = specified integer, CSS Fragmentation Module
    // Level 3 §3.3) — any positive value is "worst case" (`Direction`
    // sibling comment above uses the same reasoning).
    Orphans => PropertyValue::Orphans(5),
    Widows => PropertyValue::Widows(7),
    Custom => PropertyValue::CustomProperty(CustomProperty {
        name: "--sample".into(),
        value: "1px".into(),
    }),
    // Font-relative lengths exercise the page phase-3 conversion for the
    // supported static-side properties; percentage radii remain symbolic
    // through this page conversion.
    BorderRadius => PropertyValue::BorderRadius(BorderRadius {
        top_left: Length::Em(0.5),
        top_right: Length::Rem(0.25),
        bottom_right: Length::Pt(6.0),
        bottom_left: Length::Px(1.0),
    }),
    BorderRadiusTopLeft => PropertyValue::BorderRadiusTopLeft(Length::Em(0.5)),
    BorderRadiusTopRight => PropertyValue::BorderRadiusTopRight(Length::Rem(0.25)),
    BorderRadiusBottomRight => PropertyValue::BorderRadiusBottomRight(Length::Pt(6.0)),
    BorderRadiusBottomLeft => PropertyValue::BorderRadiusBottomLeft(Length::Px(1.0)),
    BoxShadow => PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
        offset_x: Length::Em(0.5),
        offset_y: Length::Rem(0.25),
        blur_radius: Length::Pt(3.0),
        spread_radius: Length::Px(1.0),
        color: TextShadowColor::Resolved(GREEN),
        inset: false,
    }])),
    Outline => PropertyValue::Outline(Outline {
        width: Length::Em(0.25),
        style: OutlineStyle::Solid,
        color: OutlineColor::Resolved(GREEN),
    }),
    OutlineWidth => PropertyValue::OutlineWidth(Length::Em(0.25)),
    OutlineStyle => PropertyValue::OutlineStyle(OutlineStyle::Solid),
    OutlineColor => PropertyValue::OutlineColor(OutlineColor::Resolved(GREEN)),
    OutlineOffset => PropertyValue::OutlineOffset(Length::Em(0.5)),
    // `VerticalRl` is deliberately the "worst case" here — it is one of
    // the 4 keywords `resolve_writing_mode` actually rewrites (->
    // `HorizontalTb`), same reasoning as `Overflow`'s `Visible` sample
    // above. Unlike `Direction`, `writing-mode` *does* have a
    // specified/computed distinction in this crate (`WritingMode` doc's
    // Non-goal section) — picking `HorizontalTb` here would make it
    // indistinguishable from a pass-through and silently defeat the
    // phase-3 transform classification this corpus drives. Placed last
    // to match `PropertyKey`'s own declaration order (`property.rs`),
    // per this macro's `property_key_samples!` doc contract.
    // Future work: vertical writing-mode 実装時に computed
    // value が specified value を保持するようになったら、本 sample の
    // worst-case 理由付けと `KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE` を
    // 同時に見直すこと。
    WritingMode => PropertyValue::WritingMode(WritingMode::VerticalRl),
    RubyPosition => PropertyValue::RubyPosition(RubyPosition::Over),
    // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8 — keyword-only,
    // carry no length (`x`/`y` intentionally asymmetric to catch a
    // swapped-axis regression, same reasoning as `OverflowXY`'s
    // distinct x/y samples).
    BackgroundRepeat => PropertyValue::BackgroundRepeat(BackgroundRepeat {
        x: BackgroundRepeatKeyword::Round,
        y: BackgroundRepeatKeyword::Space,
    }),
    BackgroundAttachment => PropertyValue::BackgroundAttachment(BackgroundAttachment::Fixed),
    BackgroundClip => PropertyValue::BackgroundClip(VisualBox::ContentBox),
    BackgroundOrigin => PropertyValue::BackgroundOrigin(VisualBox::ContentBox),
    // CSS Backgrounds and Borders 3 §2.9 — `Length::Em` worst-case
    // payload (same convention as `Padding`/`Width` above) so this raw
    // sample is caught by `specified_layer_residue` before phase 3
    // absolutizes it; `height: Auto` also exercises the "second axis
    // omitted defaults to auto" fill rule's `Auto` arm.
    BackgroundSize => PropertyValue::BackgroundSize(BackgroundSize::Explicit {
        width: LengthOrAuto::Length(Length::Em(2.0)),
        height: LengthOrAuto::Auto,
    }),
    // CSS Backgrounds and Borders 3 §2.6 — `Em`/`Rem` worst-case
    // payload on both `Start` and `End` (distinct edges, to exercise
    // both `CssPositionOffset` variants in the same sample).
    BackgroundPosition => PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Em(1.0)),
        vertical: CssPositionOffset::End(Length::Rem(2.0)),
    }),
    // CSS Backgrounds and Borders 3 §2.3 — `Url` is the non-initial
    // worst case among the two payloads with no length component
    // (`None` is the spec initial value, same reasoning as
    // `WritingMode`'s `VerticalRl` sample above). The third payload,
    // `Gradient(..)` (CSS Images 4 §3), *does* carry length/angle
    // components, but isn't a more interesting sample for this
    // detector — `specified_layer_residue`'s `BackgroundImage` arm
    // reports `None` unconditionally regardless of payload (that arm's
    // doc explains why), so `Gradient(..)` wouldn't exercise anything
    // `Url` doesn't already.
    BackgroundImage => PropertyValue::BackgroundImage(BackgroundImage::Url(
        "marble.svg".to_string(),
    )),
    // CSS Backgrounds and Borders 3 §2.10 — shorthand fall-through
    // (`Flex`/`Gap`/`Border` above use the same "sample a shorthand with
    // a length-bearing component" shape). Only `position`/`size` carry a
    // length; the other 6 components reuse non-initial keyword/color
    // payloads distinct from their standalone longhand samples above, to
    // catch a field-swap regression in the shorthand's own fall-through
    // arms.
    Background => PropertyValue::Background(BackgroundShorthand {
        color: GREEN,
        image: BackgroundImage::Url("tile.png".to_string()),
        repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Space,
            y: BackgroundRepeatKeyword::Round,
        },
        attachment: BackgroundAttachment::Local,
        position: CssPosition {
            horizontal: CssPositionOffset::Start(Length::Em(1.5)),
            vertical: CssPositionOffset::End(Length::Rem(0.5)),
        },
        size: BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Em(3.0)),
            height: LengthOrAuto::Auto,
        },
        clip: VisualBox::PaddingBox,
        origin: VisualBox::ContentBox,
    }),
    // CSS Images Module Level 3 §5.1 — keyword-only, carries no length.
    // `Contain` is the non-initial worst case (`fill` is the spec
    // initial, same reasoning as `BackgroundAttachment`'s `Fixed`
    // sample above).
    ObjectFit => PropertyValue::ObjectFit(ObjectFit::Contain),
    // CSS Images Module Level 3 §5.2 — `Em`/`Rem` worst-case payload on
    // both `Start` and `End` (distinct edges), same convention as
    // `BackgroundPosition` above; type itself (`CssPosition`) is
    // reused verbatim.
    ObjectPosition => PropertyValue::ObjectPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Em(2.0)),
        vertical: CssPositionOffset::End(Length::Rem(1.0)),
    }),
    // CSS Color 4 §3.3 — worst case is an out-of-range value (`2.0`,
    // not just non-initial), so this sample actually exercises the
    // `Opacity` arm's `[0, 1]` clamp in
    // `phase_3_variant_classification_matches_the_documented_counts`
    // (that test counts corpus entries where `absolutize_in_page_context`
    // is a no-op; `2.0` clamps to `1.0` and is therefore correctly
    // *not* counted as pass-through — an in-range sample like `0.5`
    // would clamp to itself and wrongly inflate
    // `PHASE_3_PASS_THROUGH_VARIANTS`, same load-bearing-fixture
    // convention as that test's own `Solid`/`Hidden` choices).
    Opacity => PropertyValue::Opacity(2.0),
    // CSS Compositing and Blending Level 1 §3.4.2 — non-initial
    // (`isolate`, not `auto`) so a would-be pass-through regression
    // (accidentally routing this arm through a transform) is visible.
    Isolation => PropertyValue::Isolation(Isolation::Isolate),
    // CSS Compositing and Blending Level 1 §3.4.1 — non-initial
    // (`multiply`, not `normal`), same rationale as `Isolation` above.
    MixBlendMode => PropertyValue::MixBlendMode(MixBlendMode::Multiply),
    // CSS Masking Level 1 §7.1 — non-initial (`Url`, not `None`).
    MaskImage => PropertyValue::MaskImage(MaskImage::Url("mask.svg".to_string())),
    // CSS Masking Level 1 §5.1 — non-initial (`GeometryBox`, not
    // `None`).
    ClipPath => PropertyValue::ClipPath(ClipPath::GeometryBox(GeometryBox::PaddingBox)),
    // CSS Transforms Level 1 §4 — non-initial (a `translate()` with
    // an `Em`/`Percent` payload, not empty-list `none`) so this
    // sample would exercise a would-be phase-3 absolutization of the
    // `Em` half if one existed — none does yet (`TransformFunction`
    // doc's "Absolutization gap" section).
    Transform => PropertyValue::Transform(Arc::new(vec![TransformFunction::Translate(
        Length::Em(2.0),
        Length::Percent(50.0),
    )])),
    // CSS Filter Effects Level 1 §5 — non-initial (a `blur()` with an
    // `Em` payload), same rationale as `Transform` above.
    Filter => PropertyValue::Filter(Arc::new(vec![FilterFunction::Blur(Length::Em(1.0))])),
    Top => PropertyValue::Top(LengthOrAuto::Auto),
    Right => PropertyValue::Right(LengthOrAuto::Auto),
    Bottom => PropertyValue::Bottom(LengthOrAuto::Auto),
    Left => PropertyValue::Left(LengthOrAuto::Auto),
    // CSS Tables 3 §4 table-layout — non-initial (`fixed`, not `auto`)
    // so a would-be pass-through regression (accidentally routing this
    // arm through a transform) is visible (`Isolation` sibling comment
    // above uses the same rationale).
    TableLayout => PropertyValue::TableLayout(TableLayoutValue::Fixed),
    // CSS Tables 3 §6 border-collapse — non-initial (`collapse`, not
    // `separate`), same rationale as `TableLayout` above.
    BorderCollapse => PropertyValue::BorderCollapse(BorderCollapseValue::Collapse),
    // CSS Tables 3 §6.1 border-spacing — `Em` (not `Px`), same "worst
    // case" reasoning as `LetterSpacing`/`TabSize` above: a font-relative
    // unit exercises phase-3 absolutization (`resolve_border_spacing`)
    // instead of trivially round-tripping an already-absolute length.
    // Two axes differ so the sample is not shortest-serializable either.
    BorderSpacing => PropertyValue::BorderSpacing(BorderSpacingValue {
        horizontal: Length::Em(0.5),
        vertical: Length::Rem(0.75),
    }),
    // CSS Tables 3 §7 caption-side — non-initial (`bottom`, not `top`),
    // same rationale as `TableLayout` above.
    CaptionSide => PropertyValue::CaptionSide(CaptionSideValue::Bottom),
    // CSS Tables 3 §8 empty-cells — non-initial (`hide`, not `show`),
    // same rationale as `TableLayout` above.
    EmptyCells => PropertyValue::EmptyCells(EmptyCellsValue::Hide),
    // CSS Fonts 4 §2.1 — shorthand fall-through (`Background` above
    // uses the same "sample a shorthand with a length-bearing
    // component" shape). `size`/`line-height` carry the lengths; the
    // other 4 components reuse non-initial payloads distinct from
    // their standalone longhand samples above, to catch a field-swap
    // regression in the shorthand's own fall-through arms.
    Font => PropertyValue::Font(FontShorthand {
        style: FontStyle::Italic,
        variant: FontVariantCaps::SmallCaps,
        weight: FontWeightValue::Absolute(700.0),
        size: FontShorthandSize::Absolute(Length::Em(1.5)),
        line_height: LineHeight::Number(1.5),
        family: Arc::new(vec![Atom::from("serif")]),
    }),
    // CSS Paged Media 3 §8.1 — keyword-only, carries no length.
    // `Named` is the non-initial worst case (`Auto` is the spec
    // initial, same reasoning as `BackgroundAttachment`'s `Fixed`
    // sample above).
    Page => PropertyValue::Page(PageValue::Named(Atom::from("cover"))),
    ListStyleType => PropertyValue::ListStyleType(ListStyleType::Named("decimal".into())),
    ListStylePosition => PropertyValue::ListStylePosition(ListStylePosition::Inside),
    ListStyleImage => PropertyValue::ListStyleImage(BackgroundImage::Url("marker.png".into())),
    ColumnCount => PropertyValue::ColumnCount(ColumnCountValue::Count(3)),
    ColumnWidth => PropertyValue::ColumnWidth(ColumnWidthValue::Length(Length::Em(2.0))),
    Columns => PropertyValue::Columns(ColumnsShorthand {
        width: ColumnWidthValue::Length(Length::Em(2.0)),
        count: ColumnCountValue::Count(3),
    }),
}

/// `sample_for` の 1:1 `PropertyKey -> PropertyValue` マッピングに
/// **乗らない** `PropertyValue` variant — 他の variant と `PropertyKey`
/// を意図的に共有するもの。今日時点でこれに該当するのは
/// [`PropertyValue::FontSizeRelative`] (`PropertyKey::FontSize` を
/// `PropertyValue::FontSize` と共有 — cascade winner selection のための
/// 設計、同 variant の doc 参照) だけ。
///
/// `page_corpus` へは**この関数の戻り値をそのまま追加**する — 「+1」の
/// ような長さの算術に畳まない。2 つ目の key 共有 variant が現れたら
/// ここに `vec!` の要素をもう 1 つ足すだけで済み、この comment を
/// 読み解いて hard-coded value を計算し直す必要が無い。
///
/// 沿革: 当初は空 (共有 pattern 自体が無かった)、
/// `FontSizeRelative` が追加されて以来 1 要素のまま変わっていない。
///
/// 2 つ目の key 共有 variant を足す義務は、以前は comment 頼みだった
/// (compile error による forcing が無かった)。
/// 今は `property_value_variant_registry!` (下) が `PropertyValue` 自身に
/// 対して網羅的な match を生成しており、新 variant を足すとまずそちらが
/// compile error になる。その状態で本関数への追加を忘れても
/// `page_corpus_covers_every_registered_property_value_variant` (test、
/// 下) が red になるので、ここへの追加漏れは最終的に検出される。
fn key_sharing_extras() -> Vec<PropertyValue> {
    use crate::property::{DeferredValue, RelativeFontSize};
    vec![
        PropertyValue::FontSizeRelative(RelativeFontSize::Larger),
        PropertyValue::CounterResetInherit,
        PropertyValue::MarginTopInherit,
        PropertyValue::MarginRightInherit,
        PropertyValue::MarginBottomInherit,
        PropertyValue::MarginLeftInherit,
        PropertyValue::MarginInherit,
        PropertyValue::BorderRadiusInherit,
        PropertyValue::Deferred(DeferredValue {
            property: "width".into(),
            value: "calc(1px + 1px)".into(),
            key: PropertyKey::Width,
        }),
        PropertyValue::CalcLengthPercentage {
            key: PropertyKey::Width,
            value: crate::property::CalcLengthPercentage {
                percent: 5.0,
                px: 10.0,
            },
        },
    ]
}

/// 全 `PropertyValue` variant を **specified 層の worst case** payload で
/// 1 つずつ並べたもの — `sample_for` (`ALL_PROPERTY_KEYS` を経由) と
/// `key_sharing_extras` から生成する。並び順は
/// `PropertyKey` の宣言順 + 末尾に key 共有 variant。本 module のどの
/// test も corpus の順序には依存しない (`HashSet` / `filter` / 走査で
/// 完結する) ので、`PropertyValue` 自身の宣言順 (旧来の順序) との違いは
/// 挙動に影響しない。
///
/// worst case = 「phase 2 / phase 3 を通さなければ specified 層の残滓が
/// 残る」値: length は `Em` / `Rem` / `Pt` (`Px` / `Percent` は既に computed
/// 層なので使わない)、`font-weight` は `bolder`、`text-align` は
/// `match-parent`、`font-size` の relative variant は `larger`。
/// 個々の選定根拠は `sample_for` / `key_sharing_extras`
/// の呼び出し箇所を参照。
///
/// この関数**自体**の完全性 (「`PropertyValue` の全 variant を実際に
/// 覆っているか」) は `ALL_PROPERTY_KEYS` / `sample_for` の網羅性からは
/// 出てこない (`sample_for` は `PropertyKey` に対して網羅的であり、
/// `PropertyValue` に対してではない)。その完全性は
/// `property_value_variant_registry!` + `page_corpus_covers_every_registered_property_value_variant`
/// (共に下) が別途保証する。
fn page_corpus() -> Vec<PropertyValue> {
    let mut corpus: Vec<PropertyValue> =
        ALL_PROPERTY_KEYS.iter().copied().map(sample_for).collect();
    corpus.extend(key_sharing_extras());
    corpus
}

/// `PropertyValue` **自身**に対して網羅的な match を 1 つの token 列から
/// 生成する (`property_key_samples!` の姉妹 macro)。
///
/// `property_key_samples!` は `PropertyKey` に対して網羅的なので、新しい
/// `PropertyKey` を伴う通常の property 追加は forced だが、**既存の**
/// `PropertyKey` を再利用する新 variant (`FontSizeRelative` が
/// `PropertyKey::FontSize` を再利用するのと同型) は `PropertyKey` の
/// variant 集合を増やさないため、その網羅 match は compile error に
/// ならない (`page_corpus` 手前の section comment の「一方向性」節で
/// 述べた、その rework が残した follow-up として追跡していた
/// ギャップ)。
///
/// 本 macro はこの穴を埋める — `PropertyValue` 自身の variant 集合に
/// 対して網羅的なので、key を再利用する variant も含め **どんな新
/// variant でも** compile error になる。`stable Rust` に variant を
/// 安全に列挙する手段 (`mem::variant_count` は unstable、外部 derive
/// crate は 独立実装方針外) が無いという前提は変わっていないが、
/// 「型に対する reflection」ではなく「型に対する網羅 match」で同じ
/// forcing を得られる — これは `property_key_samples!` が `PropertyKey`
/// に対して既にやっていることを `PropertyValue` に一般化しただけであり、
/// `PropertyValue`/`PropertyKey` の定義自体には一切触れない
/// (`#[non_exhaustive]` は downstream crate にのみ効くため、定義側と
/// 同じ crate 内の本 match には影響しない)。
///
/// 生成するもの:
/// - `PROPERTY_VALUE_VARIANT_COUNT`: token 列の要素数 = 現在の
///   `PropertyValue` variant 総数。
/// - `property_value_variant_name`: 上記の網羅 match。戻り値
///   (variant 名の文字列) 自体に意味は無い — exhaustiveness を
///   compile-time に強制することだけが目的。
///
/// この 2 つを組み合わせても、`key_sharing_extras()` / `sample_for` への
/// 追加漏れそのものを**この macro だけでは**検出しない — 網羅 match は
/// 「型に新しい variant が増えた」ことだけを compile error にする。
/// 「増えた variant を corpus (`page_corpus`) 側へ反映し忘れた」ことは
/// `page_corpus_covers_every_registered_property_value_variant` (test、
/// 下) が runtime で検出する: `PROPERTY_VALUE_VARIANT_COUNT` は compile
/// error 経由で正しく増えるが `page_corpus().len()` は反映漏れがあれば
/// 増えないので、両者の不一致が test failure として現れる。
macro_rules! property_value_variant_registry {
        ($($variant:ident),+ $(,)?) => {
            const PROPERTY_VALUE_VARIANT_COUNT: usize = [$(stringify!($variant)),+,
                "CounterResetInherit", "MarginTopInherit", "MarginRightInherit",
                "MarginBottomInherit", "MarginLeftInherit", "MarginInherit",
                "BorderRadiusInherit", "CalcLengthPercentage", "GridArea", "Grid"].len();

            fn property_value_variant_name(value: &PropertyValue) -> &'static str {
                match value {
                    PropertyValue::CounterResetInherit => "CounterResetInherit",
                    PropertyValue::MarginTopInherit => "MarginTopInherit",
                    PropertyValue::MarginRightInherit => "MarginRightInherit",
                    PropertyValue::MarginBottomInherit => "MarginBottomInherit",
                    PropertyValue::MarginLeftInherit => "MarginLeftInherit",
                    PropertyValue::MarginInherit => "MarginInherit",
                    PropertyValue::BorderRadiusInherit => "BorderRadiusInherit",
                    PropertyValue::CalcLengthPercentage { .. } => "CalcLengthPercentage",
                    PropertyValue::GridArea(_) => "GridArea",
                    PropertyValue::Grid(_) => "Grid",
                    $(PropertyValue::$variant(_) => stringify!($variant),)+
                }
            }
        };
    }

// `property.rs` の `PropertyValue` 宣言順と同じ順に列挙 (機械的な追従を
// 楽にするための慣習 — 順序自体に意味は無い、`property_key_samples!` の
// 呼び出しと同様)。
property_value_variant_registry! {
    CustomProperty,
    Deferred,
    Color,
    BackgroundColor,
    FontFamily,
    FontSize,
    FontSizeRelative,
    FontWeight,
    LineHeight,
    Display,
    ListStyleType,
    ListStylePosition,
    ListStyleImage,
    CounterReset,
    CounterIncrement,
    CounterSet,
    Content,
    StringSet,
    Position,
    TextAlign,
    HangingPunctuation,
    TextIndent,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    Padding,
    PaddingInline,
    PaddingBlock,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Margin,
    MarginInline,
    MarginBlock,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    Border,
    BorderStyle,
    BorderWidth,
    BorderColor,
    Width,
    Height,
    MaxWidth,
    MaxHeight,
    MinWidth,
    MinHeight,
    MinBlockSize,
    TextUnderlineOffset,
    BoxSizing,
    Direction,
    OverflowX,
    OverflowY,
    Overflow,
    TextDecorationLine,
    TextDecorationStyle,
    TextDecorationColor,
    TextDecoration,
    VerticalAlign,
    FontStyle,
    TextTransform,
    Visibility,
    ZIndex,
    WordBreak,
    OverflowWrap,
    LetterSpacing,
    WordSpacing,
    BreakBefore,
    BreakAfter,
    BreakInside,
    Float,
    Clear,
    WhiteSpace,
    TextWrap,
    FlexDirection,
    FlexWrap,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    FlexFlow,
    Order,
    JustifyContent,
    AlignContent,
    AlignItems,
    AlignSelf,
    RowGap,
    ColumnGap,
    Gap,
    PlaceContent,
    Hyphens,
    TabSize,
    LineBreak,
    TextJustify,
    TextAlignAll,
    TextAlignLast,
    TextCombineUpright,
    TextOrientation,
    UnicodeBidi,
    FontVariantCaps,
    Quotes,
    TextShadow,
    BorderRadius,
    BorderRadiusTopLeft,
    BorderRadiusTopRight,
    BorderRadiusBottomRight,
    BorderRadiusBottomLeft,
    BoxShadow,
    Outline,
    OutlineWidth,
    OutlineStyle,
    OutlineColor,
    OutlineOffset,
    GridTemplateColumns,
    GridTemplateRows,
    GridTemplateAreas,
    GridAutoColumns,
    GridAutoRows,
    GridAutoFlow,
    GridRowStart,
    GridRowEnd,
    GridColumnStart,
    GridColumnEnd,
    GridRow,
    GridColumn,
    JustifyItems,
    JustifySelf,
    PlaceItems,
    PlaceSelf,
    Orphans,
    Widows,
    WritingMode,
    RubyPosition,
    BackgroundRepeat,
    BackgroundAttachment,
    BackgroundClip,
    BackgroundOrigin,
    BackgroundSize,
    BackgroundPosition,
    BackgroundImage,
    Background,
    ObjectFit,
    ObjectPosition,
    Opacity,
    Isolation,
    MixBlendMode,
    MaskImage,
    ClipPath,
    Transform,
    Filter,
    TableLayout,
    BorderCollapse,
    BorderSpacing,
    CaptionSide,
    EmptyCells,
    Top,
    Right,
    Bottom,
    Left,
    Font,
    TextDecorationSkipInk,
    TextDecorationSkipSpaces,
    TextDecorationThickness,
    TextDecorationInset,
    TextEmphasisPosition,
    TextUnderlinePosition,
    Page,
    ColumnCount,
    ColumnWidth,
    Columns,
}

/// `page_corpus()` が `property_value_variant_registry!` に登録された
/// **全ての** `PropertyValue` variant を実際に覆っていること —
/// 先述の一方向性 gap のクローズ。
///
/// 新しい variant が `property_value_variant_registry!` の呼び出しに
/// 追加されないままだと `property_value_variant_name` の網羅 match が
/// compile error になる (この test 以前の問題)。本 test はその一歩先 —
/// **compile は通ったが corpus への反映を忘れた**中間状態 (`sample_for`
/// への新 `PropertyKey` arm 追加、または `key_sharing_extras()` への
/// 要素追加のどちらかを忘れた場合) を runtime で検出する。
#[test]
fn page_corpus_covers_every_registered_property_value_variant() {
    let corpus = page_corpus();
    // `property_value_variant_name` を実際の corpus 値に対して呼ぶ ---
    // この match が持つ exhaustiveness 自体は型レベルで compile-time に
    // 強制されている (呼び出し有無に関わらず) ので、ここでの呼び出しは
    // 主に「dead code にしない」ための実利用と、match 本体が実際の
    // payload shape に対して panic しないことの smoke check。
    for value in &corpus {
        property_value_variant_name(value);
    }
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        corpus.len(),
        PROPERTY_VALUE_VARIANT_COUNT,
        "page_corpus() has {} entries but property_value_variant_registry! \
             lists {} PropertyValue variants -- a variant was added to the \
             registry without a matching sample_for (new PropertyKey) or \
             key_sharing_extras() (reused PropertyKey) entry, or vice versa.",
        corpus.len(),
        PROPERTY_VALUE_VARIANT_COUNT,
    );
}

/// `value` が **specified 層でしか意味を持たない表現**を残しているか。
///
/// `PageCascadeResult::declarations` の doc が宣言する「これは computed 値だ」
/// を検査可能にしたもの。`Some(_)` は「その値は phase 2 / phase 3 を素通り
/// した」を意味する。
///
/// **wildcard arm を置かない** — `PropertyValue` に variant を足すとここで
/// compile error になる。この compile error と `page_corpus`
/// (`sample_for`) 側の更新がどう連動する (しない) かの canonical な
/// 記述は `page_corpus` 手前の section comment に
/// ある。
///
/// # 網羅 match が及ぶ payload 型は 5 つだけ
///
/// `Length` / `LengthOrAuto` / `LineHeight` / `FontWeightValue` /
/// `TextAlign`。この 5 型については
/// `crate::cascade::resolve_against_inherited` の doc が「この guard が // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (non-#[test] mod-level helper doc) — rustdoc-blind, confirmed via わざと壊して確かめる (demoted from an already-linked span)
/// 守らない範囲」として挙げる **既存 variant への payload 追加**
/// (gap (a)) もここで compile error になる。
///
/// **及ばない**もの: `BorderStyle` / `BorderColor` / `DisplayValue` /
/// `PositionValue` / `BoxSizing` / `ContentComponent` などは `(_)` で捨てて
/// いる。また `PropertyValue::Border` は struct pattern ではなく field
/// access (`b.width`) で読むので、[`Border`] に length を運ぶ field を
/// 追加しても compile error にならない。
fn specified_layer_residue(value: &PropertyValue) -> Option<&'static str> {
    /// box property (`padding` / `margin` / `width` / `height` /
    /// `border-*-width`) の `<length-percentage>`。
    ///
    /// `%` は CSS Values 4 §5.5.1
    /// <https://www.w3.org/TR/css-values-4/#combine-percentages> の既定
    /// ("the computed value of a percentage is the specified percentage")
    /// どおり computed 層に残る。`Pt` を残滓とする根拠は「absolute で
    /// ない」ではない — CSS Values 4 §6.2
    /// <https://www.w3.org/TR/css-values-4/#absolute-lengths> は `pt` も
    /// absolute length に数える。根拠は同 § の "px is their canonical
    /// unit" 側であり、computed 層の運搬 shape を `Px` に正規化する
    /// raikiri の invariant である。
    fn length(l: Length) -> Option<&'static str> {
        match l {
            Length::Px(_) | Length::Percent(_) => None,
            Length::Em(_) => Some("Length::Em"),
            Length::Rem(_) => Some("Length::Rem"),
            Length::Pt(_) => Some("Length::Pt"),
            // additional font-relative / absolute
            // units。`Em` / `Rem` / `Pt` と同じ理由で残滓 (絶対化前は
            // computed 層に存在しない specified-only 表現)。
            Length::Ex(_) => Some("Length::Ex"),
            Length::Rex(_) => Some("Length::Rex"),
            Length::Ch(_) => Some("Length::Ch"),
            Length::Rch(_) => Some("Length::Rch"),
            Length::Ic(_) => Some("Length::Ic"),
            Length::Ric(_) => Some("Length::Ric"),
            Length::Cm(_) => Some("Length::Cm"),
            Length::Mm(_) => Some("Length::Mm"),
            Length::Q(_) => Some("Length::Q"),
            Length::In(_) => Some("Length::In"),
            Length::Pc(_) => Some("Length::Pc"),
            // Same reasoning as the `Em`/`Rem`/`Pt`
            // arms above: pre-absolutization these units don't exist in
            // the computed layer.
            Length::Lh(_) => Some("Length::Lh"),
            Length::Rlh(_) => Some("Length::Rlh"),
        }
    }
    /// `%` が computed 層に**残らない** position 用 — `font-size` と
    /// `line-height`。
    ///
    /// CSS Values 4 §5.5.1 の既定文が `font-size` を明示的な例外として
    /// 名指ししている ("such as in font-size, which computes its
    /// `<percentage>` values to `<length>`)。`line-height` は CSS Inline 3
    /// §5.1 <https://www.w3.org/TR/css-inline-3/#line-height-property> が
    /// "Percentages: computed relative to 1em" と規定する。実装側は
    /// `crate::resolve` の `resolve_font_size` / `resolve_line_height` が // doc-pointer-lint:ignore: opt-out-3, fn-body-local item doc (nested inside `specified_layer_residue`, itself inside #[cfg(test)] mod tests) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 両方とも `%` を絶対化しており、本 helper 以前の検出器はそれを
    /// computed 層と誤分類していた。
    fn length_absolute_only(l: Length, what: &'static str) -> Option<&'static str> {
        match l {
            Length::Percent(_) => Some(what),
            other => length(other),
        }
    }
    fn length_or_auto(l: LengthOrAuto) -> Option<&'static str> {
        match l {
            LengthOrAuto::Auto => None,
            LengthOrAuto::Calc(_) => Some("calc()"),
            LengthOrAuto::Length(l) => length(l),
        }
    }
    /// `flex-basis: content | <'width'>` — `auto`/`content` keyword は
    /// 常に無 residue (computed 層でも keyword のまま、`ComputedFlexBasis`
    /// doc 参照)、`<length-percentage>` は [`length`] に delegate。
    fn flex_basis(fb: FlexBasisValue) -> Option<&'static str> {
        match fb {
            FlexBasisValue::Auto
            | FlexBasisValue::Content
            | FlexBasisValue::MinContent
            | FlexBasisValue::MaxContent
            | FlexBasisValue::FitContent => None,
            FlexBasisValue::Length(l) => length(l),
        }
    }
    /// `vertical-align` の bare keyword は常に無 residue (computed 層でも
    /// keyword のまま)、`<length>` / `<percentage>` は [`length`] に
    /// delegate — `flex_basis` と同じ shape (`<percentage>` は phase 3 で
    /// `Px` に絶対化済みのため residue 判定上は `Px` 扱い)。
    fn vertical_align(va: VerticalAlign) -> Option<&'static str> {
        match va {
            VerticalAlign::Baseline
            | VerticalAlign::Sub
            | VerticalAlign::Super
            | VerticalAlign::Middle
            | VerticalAlign::TextTop
            | VerticalAlign::TextBottom
            | VerticalAlign::Top
            | VerticalAlign::Bottom => None,
            VerticalAlign::Length(l) => length(l),
        }
    }
    /// `letter-spacing` / `word-spacing` の `normal | <length>`. `normal`
    /// computes to zero (CSS Text 3 §7.2/§7.1) so it is never residue,
    /// same shape as `LengthOrAuto::Auto` above.
    fn length_or_normal(l: LengthOrNormal) -> Option<&'static str> {
        match l {
            LengthOrNormal::Normal => None,
            LengthOrNormal::Length(l) => length(l),
        }
    }
    /// `tab-size: <number [0,∞]> | <length [0,∞]>`. `<number>` is
    /// already computed-equivalent (CSS Text Module Level 3 §4.2:
    /// "Computed value: the specified number or absolute length" —
    /// unlike `letter-spacing`/`word-spacing`'s `normal`, there is no
    /// keyword-to-length collapse here to begin with), `<length>`
    /// delegates to [`length`].
    fn tab_size(ts: TabSize) -> Option<&'static str> {
        match ts {
            TabSize::Number(_) => None,
            TabSize::Length(l) => length(l),
        }
    }
    fn line_height(lh: LineHeight) -> Option<&'static str> {
        match lh {
            // CSS Inline 3 §5.1: `normal` / `<number>` は computed 値のまま。
            LineHeight::Normal | LineHeight::Number(_) => None,
            LineHeight::Length(l) => length_absolute_only(l, "line-height: <percentage>"),
        }
    }
    fn font_weight(fw: FontWeightValue) -> Option<&'static str> {
        match fw {
            FontWeightValue::Absolute(_) => None,
            FontWeightValue::Bolder => Some("font-weight: bolder"),
            FontWeightValue::Lighter => Some("font-weight: lighter"),
        }
    }
    fn text_align(ta: TextAlign) -> Option<&'static str> {
        match ta {
            TextAlign::Start
            | TextAlign::End
            | TextAlign::Left
            | TextAlign::Right
            | TextAlign::Center
            | TextAlign::Justify
            | TextAlign::JustifyAll => None,
            // この改修以降、`resolve_against_inherited` の
            // phase 2 が必ず解決するため、この arm に**到達すること自体が
            // bug** (かつての「唯一の文書化された例外」ではない —
            // `PageCascadeResult::declarations` の doc も参照)。`Some`
            // のまま残してあるのは意図的な tripwire: 新しい entry point が
            // phase 2 を経由し損ねた場合 (gap (b) 相当)
            // に本検出器が拾えるようにするため。raw corpus
            // (`specified_layer_residue_detector_is_not_vacuous`) はまさに
            // この「未解決の raw 値」を検査しているので、`None` に変えると
            // その negative control が意味を失う。
            TextAlign::MatchParent => Some("text-align: match-parent"),
            TextAlign::Inherit => Some("text-align: inherit"),
            TextAlign::InternalCenter => Some("text-align: -internal-center"),
        }
    }
    fn sides<T: Copy>(s: Sides<T>, f: impl Fn(T) -> Option<&'static str>) -> Option<&'static str> {
        [s.top, s.right, s.bottom, s.left].into_iter().find_map(f)
    }
    /// [`sides`] の 2-value ([`StartEnd<T>`]) sibling — `margin-inline`/
    /// `margin-block`/`padding-inline`/`padding-block` shorthand の
    /// start/end 2 component いずれかが残滓なら全体を残滓とする。
    fn start_end<T: Copy>(
        p: StartEnd<T>,
        f: impl Fn(T) -> Option<&'static str>,
    ) -> Option<&'static str> {
        f(p.start).or_else(|| f(p.end))
    }
    /// `<track-breadth>` — `min-content`/`max-content`/`auto`/`<flex>`
    /// keyword は常に無 residue (`<number>` 相当、absolutize 不要)、
    /// `<length-percentage>` は [`length`] に delegate。
    fn grid_track_breadth(b: &GridTrackBreadth) -> Option<&'static str> {
        match b {
            GridTrackBreadth::Length(l) => length(*l),
            GridTrackBreadth::Flex(_)
            | GridTrackBreadth::MinContent
            | GridTrackBreadth::MaxContent
            | GridTrackBreadth::Auto => None,
        }
    }
    /// `<inflexible-breadth>` — [`grid_track_breadth`] と同じ shape
    /// (`<flex>` variant を持たない点のみ異なる)。
    fn grid_inflexible_breadth(b: &GridInflexibleBreadth) -> Option<&'static str> {
        match b {
            GridInflexibleBreadth::Length(l) => length(*l),
            GridInflexibleBreadth::MinContent
            | GridInflexibleBreadth::MaxContent
            | GridInflexibleBreadth::Auto => None,
        }
    }
    /// `<track-size>` — `minmax()` はどちらかの side が残滓なら全体を
    /// 残滓とする。
    fn grid_track_size(s: &GridTrackSize) -> Option<&'static str> {
        match s {
            GridTrackSize::Breadth(b) => grid_track_breadth(b),
            GridTrackSize::MinMax(min, max) => {
                grid_inflexible_breadth(min).or_else(|| grid_track_breadth(max))
            }
            GridTrackSize::FitContent(l) => length(*l),
        }
    }
    /// `grid-template-columns` / `grid-template-rows`: `none` は常に無
    /// residue、track list は component (bare track と `repeat()` の
    /// 中身の両方) を走査していずれか 1 つでも残滓なら全体を残滓とする。
    fn grid_template_tracks(t: &GridTemplateTracks) -> Option<&'static str> {
        match t {
            GridTemplateTracks::None => None,
            GridTemplateTracks::List(list) => list.components.iter().find_map(|c| match c {
                GridTrackListComponent::Size(s) => grid_track_size(s),
                GridTrackListComponent::Repeat(r) => r.tracks.iter().find_map(grid_track_size),
            }),
        }
    }

    match value {
            PropertyValue::FontWeight(fw) => font_weight(*fw),
            PropertyValue::TextAlign(ta) => text_align(*ta),
            PropertyValue::HangingPunctuation(_) => None,
            PropertyValue::LineBreak(_) => None,
            PropertyValue::TextJustify(_) => None,
            PropertyValue::TextAlignAll(_) => None,
            PropertyValue::TextAlignLast(_)
            | PropertyValue::TextCombineUpright(_)
            | PropertyValue::TextOrientation(_)
            | PropertyValue::UnicodeBidi(_)
            // `page` carries no length either (`auto` / named page).
            | PropertyValue::Page(_)
            // `column-count` carries no specified-layer length. Width-bearing
            // forms are resolved by `absolutize_in_page_context`, so raw
            // relative units remain visible to this detector only before
            // that phase.
            | PropertyValue::ColumnCount(_) => None,
            // cov:ignore: auto width has no length residue to report.
            PropertyValue::ColumnWidth(ColumnWidthValue::Auto) => None,
            PropertyValue::ColumnWidth(ColumnWidthValue::Length(l)) => {
                length(*l)
            }
            // cov:ignore: shorthand auto width has no length residue to report.
            PropertyValue::Columns(shorthand) => match shorthand.width {
                ColumnWidthValue::Auto => None,
                ColumnWidthValue::Length(l) => length(l),
            },
            PropertyValue::LineHeight(lh) => line_height(*lh),
            // `font-size` だけは `%` も残滓 (§5.5.1 の明示的例外)。
            PropertyValue::FontSize(l) => length_absolute_only(*l, "font-size: <percentage>"),
            // `larger` / `smaller` — `font_weight` の `Bolder`/`Lighter` と同型:
            // 常に未解決の残滓。`page_corpus` の worst-case
            // payload としても使う。
            PropertyValue::FontSizeRelative(_) => Some("font-size: larger/smaller"),
            // `font` shorthand fall-through — `size` は `FontSize` /
            // `FontSizeRelative` longhand arm と同じ delegate
            // (`length_absolute_only` / 常に残滓)、`line-height` は
            // `LineHeight` arm と同じ `line_height` delegate、`weight` は
            // `FontWeight` arm と同じ `font_weight` delegate。style /
            // variant / family は length を運ばない (各 longhand arm と
            // 同じく無 residue)。
            PropertyValue::Font(shorthand) => {
                let size_residue = match shorthand.size {
                    FontShorthandSize::Absolute(l) => {
                        length_absolute_only(l, "font-size: <percentage>")
                    }
                    FontShorthandSize::Relative(_) => Some("font-size: larger/smaller"),
                };
                size_residue
                    .or_else(|| line_height(shorthand.line_height))
                    .or_else(|| font_weight(shorthand.weight))
            }
            // `text-decoration-thickness` stores `auto`/`from-font` (never
            // residue, same as `LineHeight::Normal`) or `<length-percentage>`
            // — `Percent` stays symbolic here (TR propdef says
            // "Percentages: N/A", so unlike `font-size` there is no
            // compute-away rule to mirror with `length_absolute_only`).
            PropertyValue::TextDecorationThickness(t) => match t {
                TextDecorationThickness::Auto | TextDecorationThickness::FromFont => None,
                TextDecorationThickness::Length(l) => length(*l),
            },
            // `text-decoration-inset` stores `auto` (never residue) or 1-2
            // `<length>` (`%` never parses, `TextDecorationInset` doc).
            PropertyValue::TextDecorationInset(inset) => match inset {
                TextDecorationInset::Auto => None,
                TextDecorationInset::Lengths { start, end } => {
                    length(*start).or_else(|| length(*end))
                }
            },
            // `text-decoration` shorthand fall-through — only `thickness`
            // delegates (same `length` split as the longhand arm above);
            // line/style/color carry no length.
            PropertyValue::TextDecoration(shorthand) => match shorthand.thickness {
                TextDecorationThickness::Auto | TextDecorationThickness::FromFont => None,
                TextDecorationThickness::Length(l) => length(l),
            },
            PropertyValue::TextIndent(v) => length(v.length),
            PropertyValue::PaddingTop(l)
            | PropertyValue::PaddingRight(l)
            | PropertyValue::PaddingBottom(l)
            | PropertyValue::PaddingLeft(l)
            | PropertyValue::BorderTopWidth(l)
            | PropertyValue::BorderRightWidth(l)
            | PropertyValue::BorderBottomWidth(l)
            | PropertyValue::BorderLeftWidth(l)
            | PropertyValue::OutlineOffset(l) => length(*l),
            PropertyValue::MarginTopInherit
            | PropertyValue::MarginRightInherit
            | PropertyValue::MarginBottomInherit
            | PropertyValue::MarginLeftInherit
            | PropertyValue::MarginInherit => Some("margin: inherit"),
            PropertyValue::MarginTop(l)
            | PropertyValue::MarginRight(l)
            | PropertyValue::MarginBottom(l)
            | PropertyValue::MarginLeft(l)
            | PropertyValue::Width(l)
            | PropertyValue::Height(l)
            | PropertyValue::MaxWidth(l)
            | PropertyValue::MaxHeight(l)
            | PropertyValue::MinWidth(l)
            | PropertyValue::MinHeight(l)
            | PropertyValue::MinBlockSize(l)
            | PropertyValue::TextUnderlineOffset(l) => length_or_auto(*l),
            PropertyValue::Padding(s) => sides(*s, length),
            PropertyValue::Margin(s) => sides(*s, length_or_auto),
            PropertyValue::PaddingInline(p) | PropertyValue::PaddingBlock(p) => {
                start_end(*p, length)
            }
            PropertyValue::MarginInline(p) | PropertyValue::MarginBlock(p) => {
                start_end(*p, length_or_auto)
            }
            PropertyValue::Border(s) => sides(*s, |b: Border| length(b.width)),
            // Shorthand fall-throughs — styles/colors carry no length.
            PropertyValue::BorderStyle(_)
            | PropertyValue::BorderColor(_) => None,
            PropertyValue::BorderWidth(s) => sides(*s, length),
            PropertyValue::LetterSpacing(l) | PropertyValue::WordSpacing(l) => {
                length_or_normal(*l)
            }
            PropertyValue::TabSize(ts) => tab_size(*ts),
            PropertyValue::VerticalAlign(va) => vertical_align(*va),
            PropertyValue::FlexBasis(fb) => flex_basis(*fb),
            // Shorthand fall-through — `grow`/`shrink` carry no length,
            // `basis` gets the same `flex_basis` treatment as the
            // `FlexBasis` longhand above.
            PropertyValue::Flex(f) => flex_basis(f.basis),
            PropertyValue::RowGap(l) | PropertyValue::ColumnGap(l) => length_or_normal(*l),
            // Shorthand fall-through — either component being residue makes
            // the whole shorthand residue.
            PropertyValue::Gap(g) => length_or_normal(g.row).or_else(|| length_or_normal(g.column)),
            PropertyValue::CalcLengthPercentage { .. } => Some("calc()"),
            PropertyValue::GridArea(_) | PropertyValue::Grid(_) => None,
            PropertyValue::GridTemplateColumns(v) | PropertyValue::GridTemplateRows(v) => {
                grid_template_tracks(v)
            }
            PropertyValue::GridAutoColumns(v) | PropertyValue::GridAutoRows(v) => {
                v.iter().find_map(grid_track_size)
            }
            // 層に依存しない payload — keyword / color / ident list / counter。
            PropertyValue::Color(_)
            | PropertyValue::BackgroundColor(_)
            | PropertyValue::FontFamily(_)
            | PropertyValue::Display(_)
            | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
            | PropertyValue::ListStylePosition(_)
            | PropertyValue::CounterReset(_)
            | PropertyValue::CounterResetInherit
            | PropertyValue::CounterIncrement(_)
            | PropertyValue::CounterSet(_)
            | PropertyValue::Content(_)
            | PropertyValue::StringSet(_)
            | PropertyValue::Position(_)
        | PropertyValue::Top(_)
        | PropertyValue::Right(_)
        | PropertyValue::Bottom(_)
        | PropertyValue::Left(_)
            | PropertyValue::Direction(_)
            | PropertyValue::BorderTopStyle(_)
            | PropertyValue::BorderRightStyle(_)
            | PropertyValue::BorderBottomStyle(_)
            | PropertyValue::BorderLeftStyle(_)
            | PropertyValue::BorderTopColor(_)
            | PropertyValue::BorderRightColor(_)
            | PropertyValue::BorderBottomColor(_)
            | PropertyValue::BorderLeftColor(_)
            | PropertyValue::OutlineStyle(_)
            | PropertyValue::OutlineColor(_)
            | PropertyValue::BoxSizing(_)
            // `OverflowValue` carries no length — its
            // cross-axis coupling (`resolve_overflow`) is real phase-3 work
            // (see `absolutize_in_page_context`'s `OverflowX`/`OverflowY`
            // arms), but has nothing to do with this detector, which is
            // specifically about *length* residue. See
            // `KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE` below for where that
            // distinction is accounted for.
            | PropertyValue::OverflowX(_)
            | PropertyValue::OverflowY(_)
            | PropertyValue::Overflow(_)
            // `TextDecorationLine`/`TextDecorationStyle`/`TextDecorationColor`
            // carry no length either. The `text-decoration` shorthand itself
            // does NOT join this bucket: its `-thickness` component carries
            // `<length-percentage>`, so it gets its own residue arm below
            // (same "length-bearing fields only" shape as its
            // `absolutize_in_page_context` fall-through).
            | PropertyValue::TextDecorationLine(_)
            | PropertyValue::TextDecorationStyle(_)
            | PropertyValue::TextDecorationColor(_)
            // `text-decoration-skip-ink`/`-skip-spaces`/
            // `text-emphasis-position`/`text-underline-position` carry no
            // length either (see each type's doc).
            | PropertyValue::TextDecorationSkipInk(_)
            | PropertyValue::TextDecorationSkipSpaces(_)
            | PropertyValue::TextEmphasisPosition(_)
            | PropertyValue::TextUnderlinePosition(_)
            // `FontStyle` carries no length either, at this crate's scope
            // (`normal`/`italic`/`oblique` implemented, `oblique`'s
            // `<angle>` argument is not).
            | PropertyValue::FontStyle(_)
            // `FontVariantCaps` carries no length either.
            | PropertyValue::FontVariantCaps(_)
            // `TextTransform` carries no length either.
            | PropertyValue::TextTransform(_)
            // `Visibility` carries no length either.
            | PropertyValue::Visibility(_)
            // `ZIndexValue` carries an `<integer>`, not a length.
            | PropertyValue::ZIndex(_)
            // `WordBreak` (CSS Text 3 §5.1) carries no length either.
            | PropertyValue::WordBreak(_)
            // `OverflowWrap` (CSS Text 3 §5.4, legacy alias `word-wrap`)
            // carries no length either.
            | PropertyValue::OverflowWrap(_)
            // `BreakBetween` (CSS Fragmentation Module Level 3 §3.1,
            // legacy shorthand `page-break-before`/`page-break-after`
            // included) carries no length either.
            | PropertyValue::BreakBefore(_)
            | PropertyValue::BreakAfter(_)
            // `BreakInside` (CSS Fragmentation Module Level 3 §3.2, legacy
            // shorthand `page-break-inside` included) carries no length
            // either.
            | PropertyValue::BreakInside(_)
            // `FloatValue`/`ClearValue` (CSS2 §9.5.1/§9.5.2) carry no
            // length either.
            | PropertyValue::Float(_)
            | PropertyValue::Clear(_)
            // `WhiteSpace` (CSS Text 3 §3) carries no length either.
            | PropertyValue::WhiteSpace(_)
            // `TextWrapMode` (CSS Text 4 §5 subset) carries no length either.
            | PropertyValue::TextWrap(_)
            // `Hyphens` (CSS Text 3 §5.3) carries no length either.
            | PropertyValue::Hyphens(_)
            // `FlexDirectionValue`/`FlexWrapValue` carry no length either.
            | PropertyValue::FlexDirection(_)
            | PropertyValue::FlexWrap(_)
            // `flex-grow`/`flex-shrink` carry a bare `<number>`, not a
            // length.
            | PropertyValue::FlexGrow(_)
            | PropertyValue::FlexShrink(_)
            // `flex-flow` shorthand — neither component carries a length
            // (see `place-content` below for the same shape).
            | PropertyValue::FlexFlow(_)
            // `order` carries a bare `<integer>`, not a length.
            | PropertyValue::Order(_)
            // `ContentAlignmentValue`/`SelfAlignmentValue`/`AlignSelfValue`
            // carry no length either.
            | PropertyValue::JustifyContent(_)
            | PropertyValue::AlignContent(_)
            | PropertyValue::AlignItems(_)
            | PropertyValue::AlignSelf(_)
            // `place-content` shorthand — neither component carries a
            // length (see `TextDecoration` above for the same shape).
            | PropertyValue::PlaceContent(_)
            // `quotes` (CSS Content 3 §2.4.1) carries no length either.
            | PropertyValue::Quotes(_)
            // `GridTemplateAreasValue` carries no `<length-percentage>`
            // either — computed value is the specified string list itself
            // (`GridTemplateAreasValue` doc).
            | PropertyValue::GridTemplateAreas(_)
            // `GridAutoFlowValue`/`GridLineValue` carry no length either.
            | PropertyValue::GridAutoFlow(_)
            | PropertyValue::GridRowStart(_)
            | PropertyValue::GridRowEnd(_)
            | PropertyValue::GridColumnStart(_)
            | PropertyValue::GridColumnEnd(_)
            // `grid-row`/`grid-column` shorthand — neither longhand they
            // expand to carries a length either.
            | PropertyValue::GridRow(_)
            | PropertyValue::GridColumn(_)
            // `SelfAlignmentValue`/`AlignSelfValue` carry no length either.
            | PropertyValue::JustifyItems(_)
            | PropertyValue::JustifySelf(_)
            // `place-items`/`place-self` shorthand — same shape as
            // `place-content` above.
            | PropertyValue::PlaceItems(_)
            | PropertyValue::PlaceSelf(_)
            // `orphans`/`widows` carry an `<integer>`, not a length, same as
            // `ZIndexValue` above.
            | PropertyValue::Orphans(_)
            | PropertyValue::Widows(_)
            // `WritingMode` carries no length either. Its identity-collapse
            // to `HorizontalTb` (`resolve_writing_mode`) is real phase-3
            // work, same as `OverflowX`/`OverflowY`'s cross-axis coupling
            // above — but that collapse has nothing to do with *length*
            // residue, which is all this detector checks.
            | PropertyValue::WritingMode(_)
            | PropertyValue::RubyPosition(_)
            // `background-repeat`/`background-attachment`/`background-clip`/
            // `background-origin` carry no length either — keyword-only
            // payloads (`BackgroundSize`/`BackgroundPosition` below do carry
            // `<length-percentage>` and get their own arms).
            | PropertyValue::BackgroundRepeat(_)
            | PropertyValue::BackgroundAttachment(_)
            | PropertyValue::BackgroundClip(_)
            | PropertyValue::BackgroundOrigin(_)

            // `object-fit` (CSS Images Module Level 3 §5.1) carries no
            // length either — keyword-only payload, same shape as
            // `BackgroundRepeat` above. `object-position` (§5.2) does carry
            // `<length-percentage>` (reuses `CssPosition`) and gets its own
            // arm below, next to `BackgroundPosition`.
            | PropertyValue::ObjectFit(_)
            // `opacity` (CSS Color 4 §3.3) carries a bare `f32`, not a
            // `Length` — this detector only checks for *length* residue, so
            // it reports `None` unconditionally regardless of the value's
            // range. The `[0,1]` clamp is real phase-3 work
            // (`absolutize_in_page_context`'s `Opacity` arm), same as
            // `OverflowX`/`WritingMode` above — see
            // `KEYWORD_TRANSFORMED_WITHOUT_RAW_RESIDUE`'s doc for how that
            // is accounted for.
            | PropertyValue::Opacity(_)
            // `isolation` / `mix-blend-mode` (CSS Compositing and Blending
            // Level 1 §3.4.2/§3.4.1) carry bare keyword payloads (no
            // `Length` at all, unlike `Opacity`'s `f32`) — always `None`.
            | PropertyValue::Isolation(_)
            | PropertyValue::MixBlendMode(_)

            // `clip-path` (§5.1) — no embedded length at all (no
            // `<basic-shape>` support, `ClipPath` doc's scope note), so
            // `None` here needs no `BackgroundImage`-style caveat.
            | PropertyValue::ClipPath(_) => None,
            PropertyValue::Transform(items) => items.iter().find_map(|f| match *f {
                TransformFunction::Translate(tx, ty) => length(tx).or_else(|| length(ty)),
                TransformFunction::TranslateX(v) | TransformFunction::TranslateY(v) => length(v),
                TransformFunction::Matrix(_)
                | TransformFunction::Scale(_, _)
                | TransformFunction::ScaleX(_)
                | TransformFunction::ScaleY(_)
                | TransformFunction::Rotate(_)
                | TransformFunction::Skew(_, _)
                | TransformFunction::SkewX(_)
                | TransformFunction::SkewY(_) => None,
            }),
            | PropertyValue::Filter(_) => None,
            // `table-layout` (CSS Tables 3 §4) / `border-collapse` (CSS
            // Tables 3 §6) / `caption-side` (§7) / `empty-cells` (§8) carry
            // bare keyword payloads (no `Length` at all, unlike `Opacity`'s
            // `f32`) — always `None` (`Isolation`/`MixBlendMode` sibling arms
            // above use the same reasoning).
            | PropertyValue::TableLayout(_)
            | PropertyValue::BorderCollapse(_)
            | PropertyValue::CaptionSide(_)
            | PropertyValue::EmptyCells(_) => None,
            // `border-spacing` (CSS Tables 3 §6.1) carries two `<length>`
            // payloads — report the first specified-layer residue found
            // (`Gap`'s row-then-column arm above uses the same shape, with
            // plain `length` since `BorderSpacingValue` holds bare `Length`,
            // not `LengthOrNormal`).
            PropertyValue::BorderSpacing(v) => length(v.horizontal).or_else(|| length(v.vertical)),
            // Custom properties and deferred values are pre-computed cascade
            // representations, not page-context computed length payloads.
            | PropertyValue::CustomProperty(_)
            | PropertyValue::Deferred(_) => None,
            // `text-shadow` — each item's 3 lengths (`offset-x`/`offset-y`/
            // `blur-radius`) can carry specified-layer residue (same `length`
            // check `Padding`/`Margin` use above); `<color>` carries no
            // length (`TextShadowColor` doc).
            PropertyValue::TextShadow(shadows) => shadows.iter().find_map(|s| {
                length(s.offset_x)
                    .or_else(|| length(s.offset_y))
                    .or_else(|| length(s.blur_radius))
            }),
            // `border-radius` stores four independent `<length>` corners;
            // every non-px unit is specified-layer residue until phase 3.
            PropertyValue::BorderRadiusInherit => None,
            PropertyValue::BorderRadius(radius) => [
                radius.top_left,
                radius.top_right,
                radius.bottom_right,
                radius.bottom_left,
            ]
            .into_iter()
            .find_map(length),
            PropertyValue::BorderRadiusTopLeft(radius)
            | PropertyValue::BorderRadiusTopRight(radius)
            | PropertyValue::BorderRadiusBottomRight(radius)
            | PropertyValue::BorderRadiusBottomLeft(radius) => length(*radius),
            // `box-shadow` stores four length components per item. Colors do
            // not participate in the layer check.
            PropertyValue::BoxShadow(shadows) => shadows.iter().find_map(|s| {
                length(s.offset_x)
                    .or_else(|| length(s.offset_y))
                    .or_else(|| length(s.blur_radius))
                    .or_else(|| length(s.spread_radius))
            }),
            // `outline` has one length-bearing component; style/color are
            // already computed-equivalent.
            PropertyValue::Outline(outline) => length(outline.width),
            PropertyValue::OutlineWidth(width) => length(*width),
            // `background-size` stores a `<length-percentage [0,∞]> | auto`
            // per axis — same shape as `Width`/`Height` above
            // (`length_or_auto` delegate); `cover`/`contain` carry no
            // length.
            PropertyValue::BackgroundSize(size) => match size {
                BackgroundSize::Cover | BackgroundSize::Contain => None,
                BackgroundSize::Explicit { width, height } => {
                    length_or_auto(*width).or_else(|| length_or_auto(*height))
                }
            },
            // `background-position` stores a `<length-percentage>` per
            // edge/offset (`CssPositionOffset::Start`/`End` both wrap a
            // plain `Length` — the edge itself carries no length).
            PropertyValue::BackgroundPosition(pos) => {
                fn offset_residue(o: CssPositionOffset) -> Option<&'static str> {
                    match o {
                        CssPositionOffset::Start(l) | CssPositionOffset::End(l) => length(l),
                    }
                }
                offset_residue(pos.horizontal).or_else(|| offset_residue(pos.vertical))
            }
            // `background` shorthand fall-through — `position`/`size`/`image`
            // all carry `<length-percentage>` (same shape as `Flex`/`Gap`
            // above, but now including the gradient payload's font-relative
            // lengths — `Percent` stays symbolic, same split as
            // `BackgroundImage` below).
            PropertyValue::Background(shorthand) => {
                let image_residue = {
                    fn gradient_residue(
                        g: &crate::property::Gradient,
                        length: fn(crate::property::Length) -> Option<&'static str>,
                    ) -> Option<&'static str> {
                        let stop_residue = |s: &crate::property::GradientColorStop| {
                            s.position.and_then(length)
                        };
                        let offset_residue = |o: crate::property::CssPositionOffset| match o {
                            crate::property::CssPositionOffset::Start(l)
                            | crate::property::CssPositionOffset::End(l) => length(l),
                        };
                        match g {
                            crate::property::Gradient::Linear(lg) => {
                                lg.stops.iter().find_map(&stop_residue)
                            }
                            crate::property::Gradient::Radial(rg) => {
                                let size_residue = match rg.size {
                                    crate::property::RadialSize::Extent(_) => None,
                                    crate::property::RadialSize::Circle(l) => length(l),
                                    crate::property::RadialSize::Ellipse(a, b) => {
                                        length(a).or_else(|| length(b))
                                    }
                                };
                                size_residue
                                    .or_else(|| offset_residue(rg.position.horizontal))
                                    .or_else(|| offset_residue(rg.position.vertical))
                                    .or_else(|| rg.stops.iter().find_map(stop_residue))
                            }
                            crate::property::Gradient::Conic(cg) => offset_residue(cg.position.horizontal)
                                .or_else(|| offset_residue(cg.position.vertical)),
                        }
                    }
                    match &shorthand.image {
                        crate::property::BackgroundImage::None
                        | crate::property::BackgroundImage::Url(_) => None,
                        crate::property::BackgroundImage::Gradient(g) => gradient_residue(g, length),
                    }
                };
                let size_residue = match shorthand.size {
                    BackgroundSize::Cover | BackgroundSize::Contain => None,
                    BackgroundSize::Explicit { width, height } => {
                        length_or_auto(width).or_else(|| length_or_auto(height))
                    }
                };
                image_residue
                    .or(size_residue)
                    .or_else(|| {
                        fn offset_residue(o: CssPositionOffset) -> Option<&'static str> {
                            match o {
                                CssPositionOffset::Start(l) | CssPositionOffset::End(l) => length(l),
                            }
                        }
                        offset_residue(shorthand.position.horizontal)
                            .or_else(|| offset_residue(shorthand.position.vertical))
                    })
            }
            // `background-image` / `mask-image` — `None`/`Url(String)` carry
            // no length; `Gradient(..)`'s `<length-percentage>` payloads are
            // checked via `length` (font-relative) while `Percent` stays
            // symbolic (same split as `absolutize_in_page_context`'s new
            // gradient arms). This is the detector counterpart to those
            // arms — previously the whole variant was excluded as a false
            // positive.
            PropertyValue::BackgroundImage(img) => {
                fn gradient_residue(
                    g: &crate::property::Gradient,
                    length: fn(crate::property::Length) -> Option<&'static str>,
                ) -> Option<&'static str> {
                    let stop_residue = |s: &crate::property::GradientColorStop| s.position.and_then(length);
                    let offset_residue = |o: crate::property::CssPositionOffset| match o {
                        crate::property::CssPositionOffset::Start(l)
                        | crate::property::CssPositionOffset::End(l) => length(l),
                    };
                    match g {
                        crate::property::Gradient::Linear(lg) => lg.stops.iter().find_map(&stop_residue),
                        crate::property::Gradient::Radial(rg) => {
                            let size_residue = match rg.size {
                                crate::property::RadialSize::Extent(_) => None,
                                crate::property::RadialSize::Circle(l) => length(l),
                                crate::property::RadialSize::Ellipse(a, b) => length(a).or_else(|| length(b)),
                            };
                            size_residue
                                .or_else(|| offset_residue(rg.position.horizontal))
                                .or_else(|| offset_residue(rg.position.vertical))
                                .or_else(|| rg.stops.iter().find_map(stop_residue))
                        }
                        crate::property::Gradient::Conic(cg) => offset_residue(cg.position.horizontal)
                            .or_else(|| offset_residue(cg.position.vertical)),
                    }
                }
                match img {
                    crate::property::BackgroundImage::None | crate::property::BackgroundImage::Url(_) => None,
                    crate::property::BackgroundImage::Gradient(g) => gradient_residue(g, length),
                }
            }
            PropertyValue::MaskImage(img) => {
                fn gradient_residue(
                    g: &crate::property::Gradient,
                    length: fn(crate::property::Length) -> Option<&'static str>,
                ) -> Option<&'static str> {
                    let stop_residue = |s: &crate::property::GradientColorStop| s.position.and_then(length);
                    let offset_residue = |o: crate::property::CssPositionOffset| match o {
                        crate::property::CssPositionOffset::Start(l)
                        | crate::property::CssPositionOffset::End(l) => length(l),
                    };
                    match g {
                        crate::property::Gradient::Linear(lg) => lg.stops.iter().find_map(&stop_residue),
                        crate::property::Gradient::Radial(rg) => {
                            let size_residue = match rg.size {
                                crate::property::RadialSize::Extent(_) => None,
                                crate::property::RadialSize::Circle(l) => length(l),
                                crate::property::RadialSize::Ellipse(a, b) => length(a).or_else(|| length(b)),
                            };
                            size_residue
                                .or_else(|| offset_residue(rg.position.horizontal))
                                .or_else(|| offset_residue(rg.position.vertical))
                                .or_else(|| rg.stops.iter().find_map(stop_residue))
                        }
                        crate::property::Gradient::Conic(cg) => offset_residue(cg.position.horizontal)
                            .or_else(|| offset_residue(cg.position.vertical)),
                    }
                }
                match img {
                    crate::property::BackgroundImage::None | crate::property::BackgroundImage::Url(_) => None,
                    crate::property::BackgroundImage::Gradient(g) => gradient_residue(g, length),
                }
            }
            // `object-position` stores a `<length-percentage>` per
            // edge/offset — same shape as `background-position` above (both
            // reuse `CssPosition`).
            PropertyValue::ObjectPosition(pos) => {
                fn offset_residue(o: CssPositionOffset) -> Option<&'static str> {
                    match o {
                        CssPositionOffset::Start(l) | CssPositionOffset::End(l) => length(l),
                    }
                }
                offset_residue(pos.horizontal).or_else(|| offset_residue(pos.vertical))
            }
        }
}

/// `%` が computed 層に残らない 2 つの position を検出器が取りこぼさないこと。
///
/// `page_corpus` は `PropertyKey` あたり payload を 1 つしか持てない
/// (`sample_for` が 1 key → 1 value の関数のため) ので、この 2 payload は
/// corpus ではなく検出器を直接叩く。corpus 側の `Em` payload は据え置き
/// なので raw residue の数え上げにも影響しない。
#[test]
fn percentage_is_specified_layer_residue_on_font_size_and_line_height() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::FontSize(Length::Percent(150.0))),
        Some("font-size: <percentage>"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::LineHeight(LineHeight::Length(
            Length::Percent(150.0)
        ))),
        Some("line-height: <percentage>"),
    );
    // 対照 — box property では `%` が computed 値 (CSS Values 4 §5.5.1 既定)。
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Percent(50.0))),
        None,
    );
}

/// `specified_layer_residue`'s grid helpers (`grid_track_breadth`/
/// `grid_inflexible_breadth`/`grid_track_size`/`grid_template_tracks`)
/// are only exercised end-to-end by
/// `page_declarations_carry_no_specified_layer_residue`, whose corpus
/// fixture (`sample_for`) uses a bare `<length>` track only — this
/// drives the detector directly with the branches that fixture never
/// reaches: bare keyword breadths, `minmax()` on both sides,
/// `fit-content()`, and top-level `none`.
#[test]
fn grid_track_residue_detector_covers_minmax_fit_content_and_top_level_none() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridTemplateColumns(
            GridTemplateTracks::None
        )),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridAutoColumns(std::sync::Arc::new(vec![
            GridTrackSize::Breadth(GridTrackBreadth::MinContent),
        ]))),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridAutoRows(std::sync::Arc::new(vec![
            GridTrackSize::MinMax(GridInflexibleBreadth::Auto, GridTrackBreadth::MaxContent),
        ]))),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridAutoColumns(std::sync::Arc::new(vec![
            GridTrackSize::MinMax(
                GridInflexibleBreadth::Length(Length::Percent(10.0)),
                GridTrackBreadth::Length(Length::Px(5.0)),
            ),
        ]))),
        None,
    );
    // `minmax()`'s min side still flags residue when it does carry a
    // pre-absolutization unit — proves the detector doesn't just
    // hardcode `None` for `MinMax`.
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridAutoRows(std::sync::Arc::new(vec![
            GridTrackSize::MinMax(
                GridInflexibleBreadth::Length(Length::Em(1.0)),
                GridTrackBreadth::Flex(2.0),
            ),
        ]))),
        Some("Length::Em"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridAutoRows(std::sync::Arc::new(vec![
            GridTrackSize::FitContent(Length::Percent(20.0)),
        ]))),
        None,
    );
    // `grid_template_tracks`'s `GridTemplateTracks::List` arm must scan
    // a top-level `repeat()`'s nested tracks too, not just bare `Size`
    // components — `GridAutoColumns`/`GridAutoRows` above never route
    // through `grid_template_tracks` at all (they carry a bare
    // `Vec<GridTrackSize>`, not a `GridTemplateTracks`).
    assert_eq!(
        specified_layer_residue(&PropertyValue::GridTemplateColumns(
            GridTemplateTracks::List(std::sync::Arc::new(GridTrackList {
                line_names: vec![vec![], vec![]],
                components: vec![GridTrackListComponent::Repeat(GridTrackRepeat {
                    count: GridRepeatCount::Count(2),
                    line_names: vec![vec![], vec![]],
                    tracks: vec![GridTrackSize::Breadth(GridTrackBreadth::Length(
                        Length::Em(1.0)
                    ))],
                })],
            }))
        )),
        Some("Length::Em"),
    );
}

/// `letter-spacing: normal` / `word-spacing: normal` は残滓ではない
/// (CSS Text 3 §7.2/§7.1: "Computes to zero.") — `page_corpus` の
/// `LetterSpacing`/`WordSpacing` worst-case サンプルは常に `Length`
/// variant (`sample_for` 参照) なので `length_or_normal`'s `Normal` arm
/// は corpus 経由では exercise されない。ここで直接叩く。
#[test]
fn letter_spacing_and_word_spacing_normal_is_not_specified_layer_residue() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::LetterSpacing(LengthOrNormal::Normal)),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::WordSpacing(LengthOrNormal::Normal)),
        None,
    );
    // 対照 — `Em` は残滓 (絶対化前)。
    assert_eq!(
        specified_layer_residue(&PropertyValue::LetterSpacing(LengthOrNormal::Length(
            Length::Em(1.0)
        ))),
        Some("Length::Em"),
    );
}

/// `flex-basis: content` / `auto` は残滓ではない (`ComputedFlexBasis`
/// doc: computed 層でも keyword のまま) — sibling of
/// `letter_spacing_and_word_spacing_normal_is_not_specified_layer_residue`
/// above, same reason: `page_corpus`'s `FlexBasis` worst-case sample is
/// always a `Length` variant (`sample_for` 参照), so `flex_basis`'s
/// keyword arm isn't exercised via the corpus. `row-gap`/`column-gap:
/// normal` follow the same pattern as letter-spacing/word-spacing's
/// `normal` (CSS Box Alignment 3 §8.1: normal は残滓ではなく keyword の
/// まま).
#[test]
fn flex_basis_content_and_gap_normal_are_not_specified_layer_residue() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::FlexBasis(FlexBasisValue::Content)),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::FlexBasis(FlexBasisValue::Auto)),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::RowGap(LengthOrNormal::Normal)),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::ColumnGap(LengthOrNormal::Normal)),
        None,
    );
    // 対照 — `Em` は残滓 (絶対化前)。
    assert_eq!(
        specified_layer_residue(&PropertyValue::FlexBasis(FlexBasisValue::Length(
            Length::Em(1.0)
        ))),
        Some("Length::Em"),
    );
}

/// `background-size: cover` / `contain` は残滓ではない (CSS Backgrounds 3
/// §2.9: どちらも `<length-percentage>` を伴わない bare keyword) —
/// `page_corpus`'s `BackgroundSize` worst-case sample は常に `Explicit`
/// variant (`sample_for` 参照) なので `specified_layer_residue`'s
/// `Cover | Contain => None` arm は corpus 経由では exercise されない。
#[test]
fn background_size_cover_and_contain_are_not_specified_layer_residue() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::BackgroundSize(BackgroundSize::Cover)),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::BackgroundSize(BackgroundSize::Contain)),
        None,
    );
}

/// Sibling of `background_size_cover_and_contain_are_not_specified_layer_residue`
/// above, for the `background` shorthand's own `size` field —
/// `page_corpus`'s `Background` sample always uses the `Explicit`
/// variant (`sample_for`), so this `BackgroundSize::Cover`/`Contain`
/// arm of the shorthand's fall-through isn't exercised via the corpus.
/// Here directly.
#[test]
fn background_shorthand_size_cover_and_contain_are_not_specified_layer_residue() {
    fn shorthand_with_size(size: BackgroundSize) -> BackgroundShorthand {
        BackgroundShorthand {
            color: RED,
            image: BackgroundImage::None,
            repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            attachment: BackgroundAttachment::Scroll,
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            size,
            clip: VisualBox::BorderBox,
            origin: VisualBox::PaddingBox,
        }
    }
    assert_eq!(
        specified_layer_residue(&PropertyValue::Background(shorthand_with_size(
            BackgroundSize::Cover
        ))),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::Background(shorthand_with_size(
            BackgroundSize::Contain
        ))),
        None,
    );
}

/// Sibling of `background_shorthand_size_cover_and_contain_are_not_specified_layer_residue`
/// above, for the `font` shorthand's own `size` field —
/// `page_corpus`'s `Font` sample always uses the `Absolute` variant
/// (`sample_for`), so this `Relative` arm of the shorthand's
/// fall-through isn't exercised via the corpus. Here directly: same
/// residue string as the `FontSizeRelative` longhand arm.
#[test]
fn font_shorthand_relative_size_is_specified_layer_residue() {
    let shorthand = FontShorthand {
        style: FontStyle::Normal,
        variant: FontVariantCaps::Normal,
        weight: FontWeightValue::Absolute(400.0),
        size: FontShorthandSize::Relative(RelativeFontSize::Larger),
        line_height: LineHeight::Normal,
        family: Arc::new(vec![Atom::from("serif")]),
    };
    assert_eq!(
        specified_layer_residue(&PropertyValue::Font(shorthand)),
        Some("font-size: larger/smaller"),
    );
}

/// Sibling of `font_shorthand_relative_size_is_specified_layer_residue`
/// for the `text-decoration` family — keyword payloads (`Auto` /
/// `FromFont` thickness, `Auto` inset, `Auto`-thickness shorthand) are
/// never residue; only `<length>` is.
#[test]
fn text_decoration_keyword_payloads_are_not_specified_layer_residue() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::TextDecorationThickness(
            TextDecorationThickness::Auto
        )),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::TextDecorationInset(
            TextDecorationInset::Auto
        )),
        None,
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::TextDecoration(TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::CurrentColor,
            thickness: TextDecorationThickness::Auto,
        })),
        None,
    );
}

/// `tab-size: <number>` は残滓ではない (CSS Text Module Level 3 §4.2:
/// "Computed value: the specified number or absolute length" —
/// `<number>` はそもそも computed 層でも number のまま、`normal` の
/// ような keyword-to-length collapse を経ない) — sibling of
/// `flex_basis_content_and_gap_normal_are_not_specified_layer_residue`
/// above, same reason: `page_corpus`'s `TabSize` worst-case sample is
/// always the `Length` variant (`sample_for` 参照), so `tab_size`'s
/// `Number` arm isn't exercised via the corpus. Here directly.
#[test]
fn tab_size_number_is_not_specified_layer_residue() {
    assert_eq!(
        specified_layer_residue(&PropertyValue::TabSize(TabSize::Number(4.0))),
        None,
    );
    // 対照 — `Em` は残滓 (絶対化前)。
    assert_eq!(
        specified_layer_residue(&PropertyValue::TabSize(TabSize::Length(Length::Em(1.0)))),
        Some("Length::Em"),
    );
}

/// `vertical-align` の bare keyword は残滓ではない (computed 層でも keyword の
/// まま) — sibling of
/// `flex_basis_content_and_gap_normal_are_not_specified_layer_residue`
/// above, same reason: `page_corpus`'s `VerticalAlign` worst-case
/// sample is a `Length` variant (`sample_for` 参照), so `vertical_align`'s
/// keyword arms aren't exercised via the corpus.
#[test]
fn vertical_align_keywords_are_not_specified_layer_residue() {
    for va in [
        VerticalAlign::Baseline,
        VerticalAlign::Sub,
        VerticalAlign::Super,
        VerticalAlign::Middle,
        VerticalAlign::TextTop,
        VerticalAlign::TextBottom,
        VerticalAlign::Top,
        VerticalAlign::Bottom,
    ] {
        assert_eq!(
            specified_layer_residue(&PropertyValue::VerticalAlign(va)),
            None
        );
    }
    // 対照 — `Em` は残滓 (絶対化前)。
    assert_eq!(
        specified_layer_residue(&PropertyValue::VerticalAlign(VerticalAlign::Length(
            Length::Em(1.0)
        ))),
        Some("Length::Em"),
    );
}

/// 追加した font-relative / absolute unit も
/// `Em` / `Rem` / `Pt` と同じく「絶対化前は specified 層の残滓」として
/// 検出される (`length()` inner helper の網羅 match — 新 variant 追加は
/// compile error で強制されるが、各 arm の到達は compile では保証されない
/// ため個別に exercise する)。
#[test]
fn additional_length_units_are_specified_layer_residue() {
    // straight-line asserts (no loop + lazy custom message) so every
    // comparison is unconditionally exercised under patch coverage —
    // a `assert_eq!(.., "{l:?}")` message argument is only evaluated on
    // failure, which would leave that formatting code permanently
    // uncovered by an all-passing loop.
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Ex(1.0))),
        Some("Length::Ex"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Rex(1.0))),
        Some("Length::Rex"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Ch(1.0))),
        Some("Length::Ch"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Rch(1.0))),
        Some("Length::Rch"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Ic(1.0))),
        Some("Length::Ic"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Ric(1.0))),
        Some("Length::Ric"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Cm(1.0))),
        Some("Length::Cm"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Mm(1.0))),
        Some("Length::Mm"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Q(1.0))),
        Some("Length::Q"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::In(1.0))),
        Some("Length::In"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Pc(1.0))),
        Some("Length::Pc"),
    );
    // `lh` / `rlh` units.
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Lh(1.0))),
        Some("Length::Lh"),
    );
    assert_eq!(
        specified_layer_residue(&PropertyValue::PaddingTop(Length::Rlh(1.0))),
        Some("Length::Rlh"),
    );
}

/// `page_corpus` に重複 variant が無く、`sample_for` の各 arm が自分の
/// key と一致する `PropertyValue` を返すこと。
///
/// 以前の本 test は手で持つ `PROPERTY_VALUE_VARIANTS` (単なる数) と
/// `corpus.len()` を比較していたが、両者は互いにしか照合されておらず
/// (実際に `PropertyValue::Orphan` を足して
/// 実証)、新 variant が両方同じ数のまま corpus 外に残るケースを検出
/// できなかった。その後の rework で `PROPERTY_VALUE_VARIANTS` を廃止し
/// `page_corpus` を `sample_for` 駆動に変えたので、その旧チェックは
/// **常に真になる同語反復** (`corpus.len()` は `ALL_PROPERTY_KEYS.len() +
/// key_sharing_extras().len()` の定義から出てくる) になり、削除した。
///
/// 本 test が「corpus 完全性」自体は保証しない件の canonical な記述は
/// `page_corpus` 手前の section comment にある。本 test 自身が見ている
/// のは 2 つの**内部整合性**だけ:
///
/// - discriminant の重複が無いこと (`std::mem::discriminant` — payload の
///   trait bound に依存せず variant のみを区別する)。`PropertyValue::key()`
///   はもう使えない — `FontSizeRelative` /
///   `FontSize` が意図的に `PropertyKey::FontSize` を共有し単射性が崩れた
///   ため。
/// - `sample_for(key).key() == key` — arm の中身が自分の key と食い違って
///   いないこと (コピペミス class の検出。`property_key_samples!` マクロ
///   はこの一貫性まで保証しない — マクロは token 列を lhs/rhs にそのまま
///   展開するだけで、rhs の式が lhs の `PropertyKey` に対応する variant を
///   実際に construct しているかは見ていない)。
#[test]
fn page_corpus_has_no_duplicate_or_mismatched_samples() {
    let corpus = page_corpus();
    let discriminants: std::collections::HashSet<std::mem::Discriminant<PropertyValue>> =
        corpus.iter().map(std::mem::discriminant).collect();
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        discriminants.len(),
        corpus.len(),
        "page_corpus に同じ variant が 2 度現れている (discriminant が重複)",
    );
    for key in ALL_PROPERTY_KEYS.iter().copied() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            sample_for(key).key(),
            key,
            "sample_for({key:?}) が別の PropertyKey の value を返している",
        );
    }
}

/// `font-size` and its relative form intentionally share one property key.
#[test]
fn page_corpus_font_size_and_font_size_relative_share_one_key() {
    let corpus = page_corpus();
    let font_size_key_count = corpus
        .iter()
        .filter(|v| v.key() == PropertyKey::FontSize)
        .count();
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        font_size_key_count, 2,
        "FontSize と FontSizeRelative の 2 entry が PropertyKey::FontSize を共有するはず",
    );
}

/// Confirm that every corpus value is classified consistently by page
/// context resolution. Raw values are wrapped in the test-only inherited
/// context used by this phase.
#[test]
fn phase_3_variant_classification_matches_the_documented_counts() {
    let font_size = ComputedLength(20.0);
    let ctx = ResolveContext::new(ComputedLength(16.0));
    // `Solid` にしておかないと border-*-width が style gate で `0px` に
    // 潰れ、「変換された」判定が gate 由来か絶対化由来か区別できない。
    let styles = Sides::all(BorderStyle::Solid);
    // Both axes `hidden` — "neither visible nor
    // clip", so it always triggers the CSS Overflow 3 §3.1 coupling for
    // whichever axis the corpus sample under test is (mirrors `Solid`
    // above: chosen so "changed" is due to the coupling, not a
    // coincidence of the fixture).
    let overflow_pair = OverflowXY::both(OverflowValue::Hidden);

    let unchanged = page_corpus()
        .into_iter()
        .filter(|value| {
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(value.clone()),
                font_size,
                None,
                &ctx,
                styles,
                OutlineStyle::Solid,
                overflow_pair,
            ) == *value
        })
        .count();
    assert_eq!(
        unchanged, PHASE_3_PASS_THROUGH_VARIANTS,
        "phase 3 の pass-through arm が覆う variant 数が doc とずれた",
    );
}

/// **本節の中心 pin** — phase 2 → phase 3 を通した後、`declarations` に
/// 届く値に specified 層残滓は**一切残らない**。
///
/// `PageCascadeResult::declarations` の doc が consumer に宣言している
/// 「These are computed values, with no documented exception」そのもの。
/// この改修より前は `text-align: match-parent` が唯一の例外
/// だった (旧 test 名
/// `page_declarations_carry_exactly_one_specified_layer_residue`) — 本
/// task がそれを解消したので期待値を空 `vec![]` に変えた。例外が復活したら
/// ここで落ち、doc を直させる。
#[test]
fn page_declarations_carry_no_specified_layer_residue() {
    let root = root_with_font_size(16.0);
    let font_size = ComputedLength(20.0);
    let ctx = ResolveContext::new(root.font_size);
    let styles = Sides::all(BorderStyle::Solid);
    let overflow_pair = OverflowXY::both(OverflowValue::Hidden);

    let residues: Vec<(PropertyKey, &'static str)> = page_corpus()
        .into_iter()
        .map(|v| resolve_against_inherited(v, &root, &ctx))
        .map(|v| {
            absolutize_in_page_context(
                v,
                font_size,
                None,
                &ctx,
                styles,
                OutlineStyle::Solid,
                overflow_pair,
            )
        })
        .filter_map(|v| specified_layer_residue(&v).map(|r| (v.key(), r)))
        .collect();

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        residues,
        vec![(PropertyKey::Width, "calc()")],
        "only the deferred calc() residue may remain until used-value layout",
    );
}

/// 同じ規則を **`cascade_page` の出力そのもの** に対して確かめる。
///
/// 上の test は phase 2 ∘ phase 3 を直接合成しているので、`cascade_page`
/// が phase 3 を呼ばなくなっても落ちない。契約が書かれているのは
/// `PageCascadeResult::declarations` = `cascade_page` の戻り値なので、
/// 主語を合わせた check をもう 1 本置く。後の変更で
/// `resolve_against_inherited` → `absolutize_in_page_context` の合成順序
/// 自体は `ResolvedAgainstInherited` 型で強制されるようになったが、
/// 「本関数群を一切呼ばない新しい entry point」までは型で数え上げられない
/// (その変更は narrow しただけで close していない) ので、既存経路
/// (`cascade_page`) の integration だけでも押さえておく。
#[test]
fn cascade_page_output_carries_no_specified_layer_residue() {
    let root = root_with_font_size(16.0);
    let result = page(
        "@page { font-size: 2em; font-weight: bolder; line-height: 1.5em; \
             padding: 2em; margin: 3rem; border: 12pt solid red; \
             width: 4em; height: 5em; text-align: match-parent; direction: rtl }",
        &root,
    );
    // vacuity guard — stylesheet が黙って落ちていないこと。
    //
    // exact count で持つ。`>= N` 形だと `border` shorthand の 12 longhand が
    // 丸ごと落ちる parse regression が起きても残りで閾値を超えてしまい、
    // かつ落ちた分は residue も 0 なので本 check が素通りする。
    //
    // 27 = font-size / font-weight / line-height / text-align / width /
    // height / direction の 7 + padding 4 + margin 4 + `border` shorthand
    // の展開 12 (4 side × width / style / color)。`@page` の shorthand 展開が
    // 変わったらここが先に落ちる — 失敗時の意味: corpus stylesheet が
    // 期待通り parse / 展開されていない。
    //
    // 診断文言は (custom message ではなく) この comment 側に置く:
    // `assert_eq!` の custom message 引数は assertion 失敗時のみ評価され
    // る cold path なので、test が pass する限り自動 coverage 計測上
    // uncovered 扱いになる。
    assert_eq!(result.declarations().len(), 27);

    // `declarations` は HashMap-random 順なので sort して比較する。
    let mut residues: Vec<String> = result
        .declarations()
        .values()
        .filter_map(|v| specified_layer_residue(v).map(|r| format!("{:?}: {r}", v.key())))
        .collect();
    residues.sort();
    // この改修より前はここに `TextAlign: text-align: match-parent`
    // が 1 件残っていた (関数名が予告していた「no residue」と実際の
    // assertion が食い違っていた quirk) — 今は名前どおり空になる。
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        residues,
        Vec::<String>::new(),
        "cascade_page の出力に specified 層残滓が居る",
    );
}

/// `page_declarations_carry_no_specified_layer_residue` と
/// `cascade_page_output_carries_no_specified_layer_residue` が vacuous で
/// ないこと (negative control) — 検出器は phase 2 / phase 3 を通していない
/// **raw** 値に対しては実際に発火する (`text-align: match-parent` を含む —
/// この改修以降も raw corpus はまだ resolve 前なので、この
/// negative control 自体は変わらない)。
#[test]
fn specified_layer_residue_detector_is_not_vacuous() {
    let raw = page_corpus()
        .iter()
        .filter(|v| specified_layer_residue(v).is_some())
        .count();
    assert_eq!(
        raw,
        raw_corpus_residue_variants(),
        "corpus の worst-case payload が specified 層残滓として検出されない \
             — 検出器か corpus のどちらかが骨抜きになっている",
    );
}

/// Phase 3 must leave every computed-equivalent value untouched — the
/// pass-through arm covers `PHASE_3_PASS_THROUGH_VARIANTS` of
/// `page_corpus`'s entries (the rest are transformed) and a wrong
/// classification there would corrupt a value rather than merely leave it
/// unresolved. That count is only a stand-in for "of the `PropertyValue`
/// variants" while `page_corpus` stays complete — completeness is no
/// longer independently checked (see the section comment above
/// `page_corpus`). The counts
/// themselves are pinned by
/// `phase_3_variant_classification_matches_the_documented_counts`; this
/// test drives the same rule end-to-end through `cascade_page`.
#[test]
fn cascade_page_computed_equivalent_values_pass_phase_3_unchanged() {
    let root = root_with_weight(700.0);
    let result = page(
        "@page { color: red; font-weight: bolder; display: block; \
             box-sizing: border-box; border-top-color: red; text-align: center; \
             direction: rtl; font-style: italic; text-transform: uppercase; \
             word-break: break-all; overflow-wrap: anywhere; white-space: pre }",
        &root,
    );
    assert_eq!(color_of(&result), Some(RED));
    assert_eq!(
        result.declarations().get(&PropertyKey::FontWeight),
        Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BoxSizing),
        Some(&PropertyValue::BoxSizing(
            crate::property::BoxSizing::BorderBox
        )),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Center)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::Direction),
        Some(&PropertyValue::Direction(Direction::Rtl)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::FontStyle),
        Some(&PropertyValue::FontStyle(FontStyle::Italic)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextTransform),
        Some(&PropertyValue::TextTransform(TextTransform::Uppercase)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::WordBreak),
        Some(&PropertyValue::WordBreak(WordBreak::BreakAll)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::OverflowWrap),
        Some(&PropertyValue::OverflowWrap(OverflowWrap::Anywhere)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::WhiteSpace),
        Some(&PropertyValue::WhiteSpace(WhiteSpace::Pre)),
    );
}

/// `@page { text-align: match-parent }` now resolves against the root
/// element's computed `text-align` + `direction` —
/// this used to be the crate's one documented specified-layer exception
/// (`cascade_page_text_align_match_parent_passes_through_as_specified_value`,
/// asserting `TextAlign::MatchParent` survived unresolved). CSS Text 3
/// §6.1 `#valdef-text-align-match-parent` verbatim: "an inherited value
/// of start or end is interpreted against the parent's direction value".
#[test]
fn cascade_page_text_align_match_parent_resolves_against_root_direction() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);

    // Default root (`ComputedValues::initial()`): text-align = start,
    // direction = ltr → left.
    let ltr_root = root_with_weight(700.0);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&ltr_root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Left)),
    );

    // Root with `direction: rtl` (text-align still start) → right.
    let rtl_root = ComputedValues {
        direction: Direction::Rtl,
        ..ComputedValues::initial()
    };
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&rtl_root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Right)),
    );
}

/// The trap the doc warns about: unlike the element path's root element
/// (CSS Text 3 §6.1's "computes to start"), the page context's
/// `PageInheritance::LegacyInitialValues` L3 legacy exception is **not**
/// a "no parent" case — it substitutes `ComputedValues::initial()` as an
/// ordinary inheritance parent (text-align = start, direction = ltr) and
/// goes through the same parent-direction table, landing on `left` rather
/// than being short-circuited to `start`.
#[test]
fn cascade_page_text_align_match_parent_legacy_initial_values_not_start_shortcut() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Left)),
        "the L3 legacy-exception initial-values parent must still go through the parent-direction table, not the root-element \"computes to start\" shortcut"
    );
}

/// Page-path analogue of `cascade::tests::
/// text_align_match_parent_uses_parent_direction_not_own_direction_winner`
/// (that test's doc calls itself "the end-to-end check for the whole
/// `direction` + `text-align: match-parent` design" — this is the same
/// check for the *second, independent* resolution site,
/// `resolve_against_inherited`'s `TextAlign` arm, which the element-path
/// test cannot exercise).
///
/// `@page` here declares its own (conflicting) `direction: rtl` on the
/// page context itself. Per CSS Text 3 §6.1
/// `#valdef-text-align-match-parent` ("interpreted against **the
/// parent's** direction value"), resolution must use the *root element's*
/// `ltr`, not the page context's own `rtl`. Without this test, a
/// regression that made `resolve_against_inherited` read the page
/// context's own `direction` winner instead of `inherited.direction`
/// would flip `Left` → `Right` here while every other test in this
/// module — including the residue-count pins — stayed green (none of
/// them cross own-direction with match-parent on the page path).
#[test]
fn cascade_page_text_align_match_parent_ignores_page_context_own_direction() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { direction: rtl; text-align: match-parent }",
        Origin::Author,
    );
    // Root: text-align = start, direction = ltr (defaults).
    let root = ComputedValues::initial();
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Left)),
    );
    // The page context's own `direction: rtl` winner is unaffected — it
    // is a separate property, unrelated to the match-parent resolution.
    assert_eq!(
        result.declarations().get(&PropertyKey::Direction),
        Some(&PropertyValue::Direction(Direction::Rtl)),
    );
}

#[test]
fn cascade_page_text_align_match_parent_copies_non_start_end_root_value() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);
    let root = ComputedValues {
        text_align: TextAlign::Center,
        ..ComputedValues::initial()
    };
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::FromRoot(&root),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::TextAlign),
        Some(&PropertyValue::TextAlign(TextAlign::Center)),
    );
}

#[test]
fn cascade_page_query_default_matches_only_default_rule() {
    // Default `PageContextQuery::default()` = unnamed + all pseudos false.
    // Named / pseudo rules must not match; only `@page { … }` applies.
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: green } \
             @page :first { color: red } \
             @page cover { color: blue }",
        Origin::Author,
    );
    let result = cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    );
    assert_eq!(color_of(&result), Some(GREEN));
}

#[test]
fn cascade_page_result_default_is_empty() {
    // `PageCascadeResult::default()` is the empty result (no rules
    // matched) — used by consumers that need a placeholder.
    let r = PageCascadeResult::default();
    assert!(r.declarations().is_empty());
}

#[test]
fn cascade_page_result_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<PageCascadeResult>();
}

// Post-parse shorthand injection into `@page` exercises the same CSS
// shorthand expansion and cascade ordering rules as parsed declarations.
// CSS Cascading Level 4 §3 and §6.1 require expansion into longhands and
// ordering by origin, importance, specificity, and appearance.

/// Build a page cascade after replacing one parsed declaration value.
fn page_with_post_parse_injection(
    css: &str,
    idx: usize,
    injected: PropertyValue,
) -> PageCascadeResult {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    tree.page_rules[0].declarations[idx].value = injected;
    cascade_page(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
    )
}

/// Return distinct values for the four margin sides.
fn distinct_page_margin_sides() -> Sides<LengthOrAuto> {
    Sides {
        top: LengthOrAuto::Length(Length::Px(1.0)),
        right: LengthOrAuto::Length(Length::Px(2.0)),
        bottom: LengthOrAuto::Length(Length::Px(3.0)),
        left: LengthOrAuto::Length(Length::Px(4.0)),
    }
}

/// Return the pixel value for a margin longhand.
fn margin_px(result: &PageCascadeResult, key: PropertyKey) -> Option<f32> {
    match result.declarations().get(&key) {
        Some(
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(v)))
            | PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(v)))
            | PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(v)))
            | PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(v))),
        ) => Some(*v),
        _ => None,
    }
}

/// Return the pixel value for a padding longhand.
fn padding_px(result: &PageCascadeResult, key: PropertyKey) -> Option<f32> {
    match result.declarations().get(&key) {
        Some(
            PropertyValue::PaddingTop(Length::Px(v))
            | PropertyValue::PaddingRight(Length::Px(v))
            | PropertyValue::PaddingBottom(Length::Px(v))
            | PropertyValue::PaddingLeft(Length::Px(v)),
        ) => Some(*v),
        _ => None,
    }
}

#[test]
fn post_parse_page_margin_shorthand_before_longhand_lets_longhand_win() {
    let result = page_with_post_parse_injection(
        "@page { margin-left: 99px; margin-top: 10px }",
        0,
        PropertyValue::Margin(distinct_page_margin_sides()),
    );
    assert_eq!(
        margin_px(&result, PropertyKey::MarginTop),
        Some(10.0),
        "後方 longhand が order of appearance で勝つこと (§6.1)"
    );
    assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
    assert!(
        !result.declarations().contains_key(&PropertyKey::Margin),
        "shorthand key が独立 slot に park してはならない (silent drop の直接 pin)"
    );
}

#[test]
fn post_parse_page_margin_shorthand_after_longhand_lets_shorthand_win() {
    let result = page_with_post_parse_injection(
        "@page { margin-top: 10px; margin-left: 99px }",
        1,
        PropertyValue::Margin(distinct_page_margin_sides()),
    );
    assert_eq!(margin_px(&result, PropertyKey::MarginTop), Some(1.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
    assert!(!result.declarations().contains_key(&PropertyKey::Margin));
}

#[test]
fn post_parse_page_padding_shorthand_before_longhand_lets_longhand_win() {
    let result = page_with_post_parse_injection(
        "@page { padding-left: 99px; padding-top: 10px }",
        0,
        PropertyValue::Padding(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        }),
    );
    assert_eq!(padding_px(&result, PropertyKey::PaddingTop), Some(10.0));
    assert_eq!(padding_px(&result, PropertyKey::PaddingRight), Some(2.0));
    assert_eq!(padding_px(&result, PropertyKey::PaddingBottom), Some(3.0));
    assert_eq!(padding_px(&result, PropertyKey::PaddingLeft), Some(4.0));
    assert!(!result.declarations().contains_key(&PropertyKey::Padding));
}

#[test]
fn post_parse_page_border_shorthand_before_longhand_lets_longhand_win() {
    let result = page_with_post_parse_injection(
        "@page { border-left-width: 99px; border-top-width: 10px }",
        0,
        PropertyValue::Border(Sides {
            top: Border {
                width: Length::Px(1.0),
                style: BorderStyle::Solid,
                color: BorderColor::Resolved(RED),
            },
            right: Border {
                width: Length::Px(2.0),
                style: BorderStyle::Dashed,
                color: BorderColor::Resolved(BLUE),
            },
            bottom: Border {
                width: Length::Px(3.0),
                style: BorderStyle::Dotted,
                color: BorderColor::CurrentColor,
            },
            left: Border {
                width: Length::Px(4.0),
                style: BorderStyle::Double,
                color: BorderColor::Resolved(RED),
            },
        }),
    );
    // top.width だけ後方 longhand が勝つ。
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopWidth),
        Some(&PropertyValue::BorderTopWidth(Length::Px(10.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderRightWidth),
        Some(&PropertyValue::BorderRightWidth(Length::Px(2.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderBottomWidth),
        Some(&PropertyValue::BorderBottomWidth(Length::Px(3.0))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderLeftWidth),
        Some(&PropertyValue::BorderLeftWidth(Length::Px(4.0))),
    );
    // style / color は shorthand 由来のまま per-side に残る。
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopStyle),
        Some(&PropertyValue::BorderTopStyle(BorderStyle::Solid)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderRightStyle),
        Some(&PropertyValue::BorderRightStyle(BorderStyle::Dashed)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderBottomStyle),
        Some(&PropertyValue::BorderBottomStyle(BorderStyle::Dotted)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderLeftStyle),
        Some(&PropertyValue::BorderLeftStyle(BorderStyle::Double)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderTopColor),
        Some(&PropertyValue::BorderTopColor(BorderColor::Resolved(RED))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderRightColor),
        Some(&PropertyValue::BorderRightColor(BorderColor::Resolved(
            BLUE
        ))),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderBottomColor),
        Some(&PropertyValue::BorderBottomColor(BorderColor::CurrentColor)),
    );
    assert_eq!(
        result.declarations().get(&PropertyKey::BorderLeftColor),
        Some(&PropertyValue::BorderLeftColor(BorderColor::Resolved(RED))),
    );
    assert!(!result.declarations().contains_key(&PropertyKey::Border));
}

#[test]
fn post_parse_page_shorthand_injection_propagates_important() {
    let result = page_with_post_parse_injection(
        "@page { margin-left: 99px !important; margin-top: 10px }",
        0,
        PropertyValue::Margin(distinct_page_margin_sides()),
    );
    assert_eq!(
        margin_px(&result, PropertyKey::MarginTop),
        Some(1.0),
        "important shorthand 由来の MarginTop が normal longhand に勝つこと"
    );
    assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
    assert!(!result.declarations().contains_key(&PropertyKey::Margin));
}

#[test]
fn post_parse_page_important_longhand_survives_later_normal_shorthand() {
    let result = page_with_post_parse_injection(
        "@page { margin-top: 10px !important; margin-left: 99px }",
        1,
        PropertyValue::Margin(distinct_page_margin_sides()),
    );
    assert_eq!(
        margin_px(&result, PropertyKey::MarginTop),
        Some(10.0),
        "important longhand が後方の normal shorthand 由来 longhand に勝つこと \
             (§6.1 Origin and Importance)"
    );
    assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
    assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
    assert!(!result.declarations().contains_key(&PropertyKey::Margin));
}

#[test]
fn cascade_page_deterministic_across_10_runs() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@page { color: red } \
             @page :first { color: blue } \
             @page cover:first { color: green }",
        Origin::Author,
    );
    let query = PageContextQuery {
        page_name: Some(Atom::from("cover")),
        is_first: true,
        ..Default::default()
    };
    let baseline = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
    for _ in 0..9 {
        let run = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(
            run.declarations().get(&PropertyKey::Color),
            baseline.declarations().get(&PropertyKey::Color),
        );
    }
}
