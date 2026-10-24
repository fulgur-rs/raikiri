use super::*;
use crate::layout::test_support::{bridged_display, bridged_style};
use taffy::Rect as StyleRect; // taffy-boundary-lint:ignore: Rect<LengthPercentageAuto> is style input
use taffy::Size as StyleSize; // taffy-boundary-lint:ignore: Size<Dimension> is style input

#[test]
fn apply_computed_to_style_bridges_display_to_taffy() {
    // display bridge active — DisplayValue → taffy::Display
    // Regression check that the mapping is correct.
    //
    // The margin assertion below holds for the same reason:
    // for elements without an explicit margin, the cascade supplies initial
    // `Sides::all(Length::Px(0.0))`, translated to taffy's
    // `LengthPercentageAuto::length(0.0)`. This equals
    // `taffy::Style::default().margin` (all `Length(0.0)`).
    //
    // Likewise for the padding assertion: an unspecified padding has the initial
    // `Sides::all(Length::Px(0.0))` → taffy `LengthPercentage::length(0.0)`
    // value, matching `taffy::Style::default().padding`.
    //
    // And for the size assertion: unspecified width and height have the
    // initial `LengthOrAuto::Auto`, translated to `Dimension::auto()`, which
    // matches `taffy::Style::default().size` (`Size::auto()`).
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("display:none"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr).expect("styles");
    assert_eq!(bridged_display(&mut doc, &cr, body), Display::None);

    let default_style = <crate::LayoutStyle as Default>::default();
    assert_eq!(bridged_style(&mut doc, &cr, body).size, default_style.size);
    assert_eq!(
        bridged_style(&mut doc, &cr, body).margin,
        default_style.margin
    );
    assert_eq!(
        bridged_style(&mut doc, &cr, body).padding,
        default_style.padding
    );
}

#[test]
fn calc_handles_stay_alive_across_clone_and_relayout() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "html",
        Some("display:block;width:100px;height:100px"),
    );
    let body = doc.append_element(Some(root), "body", Some("display:block"));
    let child = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:calc(50% + 10px);height:10px;padding-left:calc(10% + 5px)"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;

    crate::layout::layout_single_page(&mut doc, &cr, page).expect("layout Ok");
    let mut clone = doc.clone();
    let clone_rules = build_rule_tree(&clone);
    let clone_cascade = cascade(&clone, &clone_rules).expect("clone cascade Ok");
    let clone_calc = bridged_style(&mut clone, &clone_cascade, child)
        .size
        .width
        .into_raw()
        .calc_value();

    crate::layout::layout_single_page(&mut doc, &cr, page).expect("original relayout Ok");
    assert_eq!(
        crate::layout::dispatch::resolve_calc(clone_calc, 100.0),
        60.0,
        "clone retains its old calc payload after the original relayout"
    );
    drop(doc);
    assert_eq!(
        crate::layout::dispatch::resolve_calc(clone_calc, 100.0),
        60.0,
        "clone owns the calc payload after the original document is dropped"
    );

    crate::layout::layout_single_page(&mut clone, &clone_cascade, page).expect("clone relayout Ok");

    let style = bridged_style(&mut clone, &clone_cascade, child);
    let width = style.size.width.into_raw();
    assert!(width.has_calc());
    assert_eq!(
        crate::layout::dispatch::resolve_calc(width.calc_value(), 100.0),
        60.0
    );
    assert!(clone.nodes[child].layout.size.width.is_finite());
}

#[test]
fn retained_calc_payloads_survive_bridge_error_until_successful_rebuild() {
    use crate::layout::test_support::{ahem_paragraph_in, ifc_ahem_fonts};
    use raikiri_style::{build_rule_tree, cascade};
    use std::sync::Arc;

    let (mut doc, _initial_cascade, _root) = ahem_paragraph_in(
        "",
        "width:calc(50% + 10px);padding-left:calc(10% + 5px)",
        "a",
    );
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());

    apply_computed_to_style(&mut doc, &cascade).expect("initial style bridge");
    let old_payload = Arc::downgrade(
        doc.box_style_data
            .calc_values
            .first()
            .expect("calc payload retained"),
    );

    doc.set_font_collection_with_limits(
        ifc_ahem_fonts(),
        shodo::limits::Limits {
            max_text_bytes: Some(0),
            ..Default::default()
        },
    );
    assert!(matches!(
        apply_computed_to_style(&mut doc, &cascade),
        Err(LayoutError::IfcLimitExceeded { .. })
    ));
    assert!(
        old_payload.upgrade().is_some(),
        "failed bridge keeps the old calc payload alive"
    );

    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    apply_computed_to_style(&mut doc, &cascade).expect("successful style bridge");
    assert!(
        old_payload.upgrade().is_none(),
        "successful bridge releases the previous calc payload prefix"
    );
}

#[test]
fn apply_computed_to_style_tracks_logical_min_block_provenance() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let avoided = doc.append_element(
        Some(body),
        "div",
        Some("min-block-size: 40px; break-inside: avoid"),
    );
    let auto = doc.append_element(
        Some(body),
        "div",
        Some("min-block-size: 40px; break-inside: auto"),
    );
    let page_only = doc.append_element(
        Some(body),
        "div",
        Some("min-block-size: 40px; break-inside: avoid-page"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr).expect("styles");
    assert!(doc.nodes[avoided].has_logical_min_block_size);
    assert!(!doc.nodes[auto].has_logical_min_block_size);
    assert!(!doc.nodes[page_only].has_logical_min_block_size);
    assert_eq!(
        bridged_style(&mut doc, &cr, avoided).min_size.height,
        LengthPercentageAuto::length(40.0)
    );
}

#[test]
fn apply_computed_to_style_bridges_direction_to_taffy() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("direction:rtl"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr).expect("styles");

    assert_eq!(
        bridged_style(&mut doc, &cr, body).direction,
        taffy::Direction::Rtl
    );
}

#[test]
fn apply_computed_to_style_preserves_authored_list_item_padding() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let inside = doc.append_element(
        Some(0),
        "li",
        Some("display: list-item; list-style-position: inside"),
    );
    let outside = doc.append_element(
        Some(0),
        "li",
        Some("display: list-item; list-style-position: outside"),
    );
    let explicit_padding = doc.append_element(
        Some(0),
        "li",
        Some("display: list-item; list-style-position: inside; padding-left: 5px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr).expect("styles");

    assert_eq!(
        bridged_style(&mut doc, &cr, inside)
            .padding
            .left
            .into_raw()
            .value(),
        0.0
    );
    assert_eq!(
        bridged_style(&mut doc, &cr, outside)
            .padding
            .left
            .into_raw()
            .value(),
        0.0
    );
    assert_eq!(
        bridged_style(&mut doc, &cr, explicit_padding)
            .padding
            .left
            .into_raw()
            .value(),
        5.0
    );
}

#[test]
fn apply_computed_to_style_bridges_margin_to_taffy() {
    // Regression check that the margin translates
    // Sides<ComputedLengthPercentageOrAuto> into taffy::Rect<LengthPercentageAuto>.
    // Cover each of the three bridge branches (Px / Percent / Auto)
    // with one case.
    //
    // `pt` in Case 4 no longer has a separate bridge branch
    // (cascade phase 3 converts it to px), but the end-to-end expected
    // value is unchanged, so keep the test.
    //
    // Test approach: each case has an independent fixture that cascades,
    // calls apply_computed_to_style, then asserts body.style.margin.
    // The inline style also checks raikiri-style's shorthand and longhand margin parsing.
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::LengthPercentageAuto;

    fn margin_for(inline: &str) -> StyleRect<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, body).margin
    }

    // Case 1: shorthand `margin: 10px 20px 30px 40px` (top/right/bottom/left)
    //   → Rect { top: 10, right: 20, bottom: 30, left: 40 } (Px unchanged).
    //   Pin the field-name mapping from Sides.top,right,bottom,left to
    //   Rect.top,right,bottom,left (to prevent a silent positional transpose).
    assert_eq!(
        margin_for("margin: 10px 20px 30px 40px"),
        StyleRect {
            top: LengthPercentageAuto::length(10.0),
            right: LengthPercentageAuto::length(20.0),
            bottom: LengthPercentageAuto::length(30.0),
            left: LengthPercentageAuto::length(40.0),
        }
    );

    // Case 2: shorthand `margin: auto` → auto() on all four sides.
    assert_eq!(
        margin_for("margin: auto"),
        StyleRect {
            top: LengthPercentageAuto::auto(),
            right: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::auto(),
            left: LengthPercentageAuto::auto(),
        }
    );

    // Case 3: longhand `margin-left: 50%` → left = percent(0.5); the other
    //   three sides retain initial 0.0 px. Pin conversion of authored CSS
    //   percentages 0–100 to taffy fractions 0.0–1.0 by dividing by 100.
    assert_eq!(
        margin_for("margin-left: 50%"),
        StyleRect {
            top: LengthPercentageAuto::length(0.0),
            right: LengthPercentageAuto::length(0.0),
            bottom: LengthPercentageAuto::length(0.0),
            left: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 4: longhand `margin-top: 10pt` → length(10 * 4/3) = length(13.333...).
    //   CSS Values 4 §6.2: `1pt = 4/3 px` (1pt=1/72in, 1in=96px → 96/72=4/3).
    //   **Cascade phase 3 performs this conversion, not the bridge**
    //   (`raikiri_style::resolve_length_percentage_or_auto`).
    //   Values have already become px when they reach the bridge. This case
    //   checks the value end to end.
    //   Keep the right-hand side as an expression for a bit-identical f32 assertion:
    //   the `13.333` literal drifts after rounding. This expression uses
    //   the `v * 4.0 / 3.0` evaluation order of `resolve::pt_to_px` itself;
    //   simplifying it changes bits because f32 arithmetic is not associative.
    //   See the docs for `resolve::pt_to_px`.
    assert_eq!(
        margin_for("margin-top: 10pt"),
        StyleRect {
            top: LengthPercentageAuto::length(10.0 * 4.0 / 3.0),
            right: LengthPercentageAuto::length(0.0),
            bottom: LengthPercentageAuto::length(0.0),
            left: LengthPercentageAuto::length(0.0),
        }
    );
}

#[test]
fn apply_computed_to_style_bridges_padding_to_taffy() {
    // Regression check that the padding translates
    // Sides<ComputedLengthPercentage> into taffy::Rect<LengthPercentage>.
    // Unlike margin, padding does not accept `auto`
    // (<length-percentage `[0,∞]`>), so **two arms**
    // (Px / Percent) exhaust the bridge.
    //
    // `pt` in Case 3 is **no longer a bridge branch**
    // (cascade phase 3 converts it to px), but the end-to-end expected
    // value is unchanged, so keep the test.
    //
    // Test approach: each case has an independent fixture that cascades,
    // calls apply_computed_to_style, then asserts body.style.padding.
    // The inline style also checks raikiri-style's shorthand and longhand padding parsing.
    use raikiri_style::{build_rule_tree, cascade};

    fn padding_for(inline: &str) -> StyleRect<LengthPercentage> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, body).padding
    }

    // Case 1: shorthand `padding: 5px 10px 15px 20px` (top/right/bottom/left)
    //   → Rect { top: 5, right: 10, bottom: 15, left: 20 } (Px unchanged).
    //   Pin the field-name mapping from Sides.top,right,bottom,left to
    //   Rect.top,right,bottom,left to prevent a silent transpose: `Sides`
    //   orders fields top,right,bottom,left; `Rect` orders left,right,top,bottom.
    assert_eq!(
        padding_for("padding: 5px 10px 15px 20px"),
        StyleRect {
            top: LengthPercentage::length(5.0),
            right: LengthPercentage::length(10.0),
            bottom: LengthPercentage::length(15.0),
            left: LengthPercentage::length(20.0),
        }
    );

    // Case 2: longhand `padding-left: 5%` → left = percent(0.05); the other
    //   three sides retain initial 0.0 px. Pin conversion of authored CSS
    //   percentages 0–100 to taffy fractions 0.0–1.0 by dividing by 100.
    assert_eq!(
        padding_for("padding-left: 5%"),
        StyleRect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::percent(0.05),
        }
    );

    // Case 3: longhand `padding-top: 3pt` → length(3 * 4/3) = length(4.0).
    //   CSS Values 4 §6.2: `1pt = 4/3 px` (1pt=1/72in, 1in=96px → 96/72=4/3).
    //   **Cascade phase 3** (`resolve_length_percentage`) performs the conversion,
    //   not the bridge.
    //   Keep the right-hand side as an expression for a bit-identical f32 assertion.
    //   (`4.0` happens to be bit-identical to 3*4/3, but the expression states the policy.)
    assert_eq!(
        padding_for("padding-top: 3pt"),
        StyleRect {
            top: LengthPercentage::length(3.0 * 4.0 / 3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

#[test]
fn apply_computed_to_style_bridges_width_to_taffy() {
    // The width translates cv.width: ComputedLengthPercentageOrAuto into
    // taffy::Style::size.width: Dimension. Check each of its three branches
    // (Px / Percent / Auto) with one case
    // (`pt` is converted to px in cascade phase 3).
    //
    // Test approach: use a **non-body element** (here `<p>`) as the fixture:
    // `<body>` is overwritten by `apply_page_content_box_to_body` later, so the
    // bridge's effect is not observable there. A separate test,
    // `apply_page_content_box_clobbers_body_width_from_bridge`, pins that behavior.
    // The inline style also checks raikiri-style's parse_width path and
    // ComputedLengthPercentageOrAuto encoding as a regression check.
    //
    // This test covers only width; the sibling test
    // `apply_computed_to_style_bridges_height_to_taffy`
    // checks the same LengthOrAuto → Dimension bridge pattern for height.
    use raikiri_style::{build_rule_tree, cascade};

    fn width_for(inline: &str) -> Dimension {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        // Put the inline style on non-body element p. Since
        // apply_page_content_box_to_body only touches body, p's style.size is observable after the bridge.
        let p = doc.append_element(Some(body), "p", Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, p).size.width
    }

    // Case 1: `width: 100px` → Dimension::length(100.0) (Px unchanged).
    assert_eq!(width_for("width: 100px"), Dimension::length(100.0));

    // Case 2: `width: auto` → Dimension::auto()
    //   (ComputedLengthPercentageOrAuto::Auto arm).
    assert_eq!(width_for("width: auto"), Dimension::auto());

    // Case 3: `width: 50%` → Dimension::percent(0.5). Pin conversion of
    //   authored CSS percent 0–100 to taffy fraction 0.0–1.0 by dividing by 100.
    assert_eq!(width_for("width: 50%"), Dimension::percent(0.5));

    // Case 4: `width: 20pt` → Dimension::length(20 * 4/3) = length(26.666...).
    //   CSS Values 4 §6.2: `1pt = 4/3 px` (1pt=1/72in, 1in=96px → 96/72=4/3).
    //   **Cascade phase 3**, not the bridge, performs the conversion.
    //   Keep the right-hand side as an expression for a bit-identical f32 assertion.
    assert_eq!(
        width_for("width: 20pt"),
        Dimension::length(20.0 * 4.0 / 3.0)
    );
}

#[test]
fn apply_computed_to_style_bridges_height_to_taffy() {
    // The height mapping extends the width mapping.
    // Check that cv.height: ComputedLengthPercentageOrAuto becomes
    // taffy::Style::size.height: Dimension. This pairs with the
    // sibling test `apply_computed_to_style_bridges_width_to_taffy`.
    // The single struct-literal assignment (Size { width, height }) should
    // write all three height branches (Px / Percent / Auto)
    // as intended.
    //
    // Test approach: use a **non-body element** (`<p>`) as the fixture:
    // the later `apply_page_content_box_to_body` also overwrites height on `<body>`,
    // so the bridge's effect is not observable there. The inline style
    // also checks raikiri-style's parse_height path and the
    // ComputedLengthPercentageOrAuto encoding as a regression check.
    //
    // The sibling width test already checks the same
    // computed_length_percentage_or_auto_to_taffy_dimension policy for pt,
    // so that case is redundant (and cascade phase 3 owns pt → px conversion).
    // Focus instead on height's three branches: preserving the auto default,
    // the `Px` path, and the `Percent` path.
    use raikiri_style::{build_rule_tree, cascade};

    fn height_for(inline: &str) -> Dimension {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        // Put the inline style on non-body element p. Since
        // apply_page_content_box_to_body only touches body, p's style.size is observable after the bridge.
        let p = doc.append_element(Some(body), "p", Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, p).size.height
    }

    // Case 1: `height: 100px` → Dimension::length(100.0) (Px unchanged).
    assert_eq!(height_for("height: 100px"), Dimension::length(100.0));

    // Case 2: `height: auto` → Dimension::auto()
    //   (ComputedLengthPercentageOrAuto::Auto arm).
    //   Pin the identity round-trip of initial `height: auto` in CSS Sizing 3 §3.1.1.
    assert_eq!(height_for("height: auto"), Dimension::auto());

    // Case 3: `height: 50%` → Dimension::percent(0.5). Pin conversion of
    //   authored CSS percent 0–100 to taffy fraction 0.0–1.0 by dividing by 100.
    assert_eq!(height_for("height: 50%"), Dimension::percent(0.5));
}

#[test]
fn apply_computed_to_style_bridges_min_size_to_taffy() {
    // Min-size half of the size limits: translate cv.min_width / cv.min_height,
    // both ComputedLengthPercentageOrAuto, into taffy::Style::min_size,
    // a Size<Dimension>. Use the same fixture pattern as the
    // sibling `apply_computed_to_style_bridges_height_to_taffy` test:
    // a non-body `<p>`. `apply_page_content_box_to_body` overwrites only size on
    // body, not min/max; this fixture stays consistent with the other
    // size tests.
    //
    // Also check the identity round-trip of initial `auto` (CSS Sizing 3 §4):
    // unspecified values match the taffy default. This guards against
    // changing the default when the property is unspecified.
    use raikiri_style::{build_rule_tree, cascade};

    fn min_for(inline: Option<&str>) -> StyleSize<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let p = doc.append_element(Some(body), "p", inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, p).min_size
    }

    // Case 1: unspecified → taffy default (initial min `auto` round-trip).
    assert_eq!(
        min_for(None),
        <crate::LayoutStyle as Default>::default().min_size
    );

    // Case 2: `min-width: 100px; min-height: 50%` → length + percent.
    assert_eq!(
        min_for(Some("min-width: 100px; min-height: 50%")),
        StyleSize {
            width: LengthPercentageAuto::length(100.0),
            height: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 3: `min-width: auto` → LengthPercentageAuto::auto() (no minimum).
    assert_eq!(
        min_for(Some("min-width: auto")).width,
        LengthPercentageAuto::auto()
    );

    // Case 4: negative values violate grammar `[0,∞]`, so the declaration is dropped; Auto remains.
    assert_eq!(
        min_for(Some("min-width: -10px")).width,
        LengthPercentageAuto::auto()
    );
}

#[test]
fn apply_computed_to_style_bridges_max_size_to_taffy() {
    // Max-size half of the size limits: translate cv.max_width / cv.max_height
    // into taffy::Style::max_size: Size<Dimension>.
    // Use the same fixture pattern as the sibling min-size test.
    //
    // CSS Sizing 3 §5 initial `none` → computed Auto placeholder →
    // `Dimension::auto()` (no maximum). Also check that unspecified values
    // match the taffy default.
    use raikiri_style::{build_rule_tree, cascade};

    fn max_for(inline: Option<&str>) -> StyleSize<LengthPercentageAuto> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let p = doc.append_element(Some(body), "p", inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, p).max_size
    }

    // Case 1: unspecified → taffy default (initial max `none` round-trip).
    assert_eq!(
        max_for(None),
        <crate::LayoutStyle as Default>::default().max_size
    );

    // Case 2: `max-width: 100px; max-height: 50%` → length + percent.
    assert_eq!(
        max_for(Some("max-width: 100px; max-height: 50%")),
        StyleSize {
            width: LengthPercentageAuto::length(100.0),
            height: LengthPercentageAuto::percent(0.5),
        }
    );

    // Case 3: calc() remains a live Taffy handle instead of becoming zero.
    assert!(
        max_for(Some("max-width: calc(100px + 1%)"))
            .width
            .into_raw()
            .is_plain_calc()
    );

    // Case 3b: an inset calc() is preserved the same way.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let p = doc.append_element(
        Some(body),
        "p",
        Some("position: relative; left: calc(50% + 1em); top: calc(10% - 2px)"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr).expect("styles");
    assert!(
        bridged_style(&mut doc, &cr, p)
            .inset
            .left
            .into_raw()
            .is_plain_calc()
    );
    assert!(
        bridged_style(&mut doc, &cr, p)
            .inset
            .top
            .into_raw()
            .is_plain_calc()
    );
    assert_eq!(
        bridged_style(&mut doc, &cr, p).inset.right,
        LengthPercentageAuto::auto()
    );

    // Case 4: `max-width: none` → LengthPercentageAuto::auto() (no maximum).
    assert_eq!(
        max_for(Some("max-width: none")).width,
        LengthPercentageAuto::auto()
    );

    // Case 5: negative values violate grammar `[0,∞]`, so the declaration is dropped; Auto remains.
    assert_eq!(
        max_for(Some("max-height: -10px")).height,
        LengthPercentageAuto::auto()
    );
}

#[test]
fn apply_computed_to_style_bridges_border_to_taffy() {
    // Regression check that the border widths translate
    // Sides<ComputedBorder> to taffy::Rect<LengthPercentage>.
    // The assertion checks the mapping end to end.
    //
    // Spec-correctness gate: if border-style is `none` or `hidden`, width
    // must be zero regardless of the specified border-width.
    // **This gate belongs upstream in `raikiri_style::resolve_border`,
    // at the computed-value stage, not in this bridge**. CSS Backgrounds 3
    // §3.3 "Line Thickness: the border-width properties"
    // <https://www.w3.org/TR/css-backgrounds-3/#border-width> states in its propdef:
    // "Computed value: absolute length, snapped as a border width; zero if
    // the border style is none or hidden" (TR version; the ED moved the
    // effect to resolved values in CSSWG Issue 11494).
    // The gate moved from used values to computed values, and the bridge's
    // `used_border_width` helper was removed. Section 3.2, "Line Patterns: the
    // border-style properties"
    // <https://www.w3.org/TR/css-backgrounds-3/#border-style> also says for `none`:
    // "No border. Color and width are ignored (i.e., the border has width
    // 0)." The two descriptions agree.
    //
    // This remains an **end-to-end** gating check: even though the gate moved
    // upstream, a `5px none red` border must not leak 5px into taffy.
    // Section numbers and quotations were checked against the spec's
    // `data-level` and text. The older "§5.2 The used values of the
    // corresponding border-*-width become 0." did not exist in css-backgrounds-3.
    //
    // Test approach: use both the four-side `border: <w> <s> <c>` shorthand
    // and longhands. Check the entire shorthand expansion → per-side
    // cascade → border width pipeline. The single-side shorthand
    // `border-top: ...` is not yet supported by the parser;
    // see computed.rs lines 228–229.
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::LengthPercentage;

    fn border_for(inline: &str) -> StyleRect<LengthPercentage> {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, body).border
    }

    // Case 1 (positive path): `border: 5px solid red` shorthand → 4 side
    //   cascades with width=5 and style=solid on every side. No gating
    //   applies (solid ≠ None/Hidden), so all four sides become length(5.0).
    //   Pin the Rect.top/right/bottom/left ↔ Sides.top/right/bottom/left field-name mapping.
    assert_eq!(
        border_for("border: 5px solid red"),
        StyleRect {
            top: LengthPercentage::length(5.0),
            right: LengthPercentage::length(5.0),
            bottom: LengthPercentage::length(5.0),
            left: LengthPercentage::length(5.0),
        }
    );

    // Case 2 (spec correctness): `border: 5px none red` shorthand
    //   → width=5, style=None on all four sides. Style gating under §3.2
    //   and the §3.3 propdef (TR; quoted above) sets each computed
    //   width to zero, yielding length(0.0) on all four sides. If gating
    //   breaks, 5.0 leaks through; this case is the canary.
    assert_eq!(
        border_for("border: 5px none red"),
        StyleRect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 3 (spec correctness): `border: 5px hidden red`
    //   shorthand → computed width = 0. The direct basis is the §3.3
    //   propdef (TR), which names `hidden` alongside `none`. Section 3.2 says:
    //   "Same as none, but has different behavior in the border conflict
    //   resolution rules for border-collapsed tables `CSS2`." Thus its only
    //   difference from `none` is conflict resolution for collapsed table borders.
    assert_eq!(
        border_for("border: 5px hidden red"),
        StyleRect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 4 (pt unit conversion): `border-top-width: 3pt` + solid → top
    //   only; the other three sides retain initial width=medium=3px and
    //   style=None, so gating yields length(0.0). Top is 3pt × 4/3 = 4.0 px
    //   (CSS Values 4 §6.2: 1pt = 96/72 px = 4/3 px). Keep the right-hand side as an expression for bit-identical f32.
    assert_eq!(
        border_for("border-top-width: 3pt; border-top-style: solid"),
        StyleRect {
            top: LengthPercentage::length(3.0 * 4.0 / 3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );

    // Case 5 (medium keyword): `border-top-width: medium` + solid → top =
    //   3.0 px (see `parse_border_width_side` in property.rs). Section 3.3 says:
    //   thin, medium, and thick keywords are equivalent to 1px, 3px, and
    //   5px, respectively." These are **fixed values**. Unlike CSS2.1 §8.5.1,
    //   <https://www.w3.org/TR/CSS21/box.html#border-width-properties>
    //   which says "The interpretation of the first three values depends on the
    //   user agent," the newer spec fixes them. The other three sides are
    //   zero after gating, as in Case 4. Pin the parsing of `medium` to
    //   Length::Px(3.0) end to end.
    assert_eq!(
        border_for("border-top-width: medium; border-top-style: solid"),
        StyleRect {
            top: LengthPercentage::length(3.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

/// `ComputedBorder::width` / `::style` were narrowed from `pub` fields
/// to `pub(crate)` fields with read-only accessors to maintain this
/// invariant; this is an end-to-end pin.
///
/// Declare only `border-top-width`, not `border-top-style`.
/// The undeclared computed style remains the initial `none` (CSS Backgrounds 3
/// §3.2). This simple input checks that the declared width reaches taffy
/// as **zero** after passing through the bridge. Before narrowing, a consumer
/// could bypass this gate by taking `ComputedBorder` from
/// `ComputedValues::initial()` or similar, directly setting
/// `.style = BorderStyle::None`, and leaking a nonzero `.width` to taffy
/// (after the `used_border_width` bridge helper was removed). Narrowing
/// blocks this mutation from outside the crate for `width` / `style` only;
/// `color` remains public, and whole-side assignment such as
/// `cv.border.top = cv.border.left` is still possible. Neither breaks this invariant.
///
/// **This test checks that one input observable before narrowing is gated
/// correctly; it does not itself exhaustively prove that no outside-crate
/// path can break the invariant.** The latter is already prevented
/// **structurally by the type's visibility boundary**: `width` and
/// `style` are `pub(crate)`, and the public API offers only by-value,
/// read-only accessors (`width()` / `style()`), with no setter, builder, or
/// constructor. Safe code outside the crate cannot write either field.
///
/// **The remaining problem is different: regression detection.** If someone
/// changes `width` / `style` back from `pub(crate)` to `pub`, retracting
/// that type-level guarantee, no current test would catch it.
/// A compile-fail test setup for the other three fields (such as
/// `Declaration::value`) has already been added and merged into main,
/// but it has not yet been extended to `ComputedBorder::width` / `::style`.
#[test]
fn border_width_alone_without_declared_style_reaches_taffy_as_zero() {
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::LengthPercentage;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        // Never declare `border-style`: all four computed sides retain
        // the initial `none` (CSS Backgrounds 3 §3.2 "Inherited: no").
        Some("border-top-width: 5px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr).expect("styles");

    // border-style unset (initial `none`) must gate width to 0 all the way to taffy.
    assert_eq!(
        bridged_style(&mut doc, &cr, body).border,
        StyleRect {
            top: LengthPercentage::length(0.0),
            right: LengthPercentage::length(0.0),
            bottom: LengthPercentage::length(0.0),
            left: LengthPercentage::length(0.0),
        }
    );
}

#[test]
fn font_relative_lengths_reach_taffy_as_real_pixels() {
    // The old bridge accepted the specified-value `Length` and
    // silently collapsed font-relative units to 0px with
    // `Length::Em(_) | Length::Rem(_) => length(0.0)` (fail-quiet).
    // Cascade phases 2 and 3 now absolutize them, so real px reaches taffy.
    //
    // This test catches regressions that collapse values to 0.0; all
    // expected values are nonzero and derived from font-size.
    use raikiri_style::{build_rule_tree, cascade};
    use taffy::{Dimension, LengthPercentage, LengthPercentageAuto};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("font-size: 20px"));
    let body = doc.append_element(
        Some(html),
        "body",
        // font-size inherits 20px. `em` uses this node's 20px; `rem` uses
        // the root's 20px.
        Some(
            "padding: 2em; margin: 1.5rem; width: 3em; \
                 border-top-width: 0.5em; border-top-style: solid",
        ),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cr).expect("styles");
    let style = &bridged_style(&mut doc, &cr, body);

    // 2em × 20px = 40px (previously 0.0).
    assert_eq!(
        style.padding,
        StyleRect {
            top: LengthPercentage::length(40.0),
            right: LengthPercentage::length(40.0),
            bottom: LengthPercentage::length(40.0),
            left: LengthPercentage::length(40.0),
        }
    );
    // 1.5rem × 20px (root font-size) = 30px (previously 0.0).
    assert_eq!(
        style.margin,
        StyleRect {
            top: LengthPercentageAuto::length(30.0),
            right: LengthPercentageAuto::length(30.0),
            bottom: LengthPercentageAuto::length(30.0),
            left: LengthPercentageAuto::length(30.0),
        }
    );
    // 3em × 20px = 60px (previously 0.0).
    assert_eq!(style.size.width, Dimension::length(60.0));
    // 0.5em × 20px = 10px; style: solid allows it through gating
    // (previously 0.0).
    assert_eq!(style.border.top, LengthPercentage::length(10.0));
}

#[test]
fn apply_computed_to_style_bridges_box_sizing_to_taffy() {
    // Regression check that the box-sizing bridge performs the one-to-one
    // enum mapping from raikiri_style::BoxSizing to taffy::BoxSizing.
    // Confirm it end to end.
    //
    // 3 case:
    //   #1 border-box (specified)   → taffy::BoxSizing::BorderBox
    //   #2 content-box (specified)  → taffy::BoxSizing::ContentBox
    //   #3 unspecified (cascade default = raikiri-style initial = ContentBox)
    //      → taffy::BoxSizing::ContentBox
    //
    // Note: taffy 0.12 defaults to BorderBox (violating the spec), while
    // raikiri-style initially uses ContentBox (per CSS Sizing 3 §3.3).
    // In #3, the cascade seeds ContentBox from its initial value and the bridge
    // propagates it to taffy, correcting taffy's default as a side effect.
    use raikiri_style::{build_rule_tree, cascade};

    fn box_sizing_for(inline: Option<&str>) -> TaffyBoxSizing {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", inline);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("styles");
        bridged_style(&mut doc, &cr, body).box_sizing
    }

    // Case 1: border-box → taffy::BoxSizing::BorderBox
    assert_eq!(
        box_sizing_for(Some("box-sizing: border-box")),
        TaffyBoxSizing::BorderBox
    );

    // Case 2: content-box (explicit) → taffy::BoxSizing::ContentBox
    assert_eq!(
        box_sizing_for(Some("box-sizing: content-box")),
        TaffyBoxSizing::ContentBox
    );

    // Case 3: unspecified → raikiri-style initial (ContentBox) → taffy ContentBox
    // (overwrites taffy's default BorderBox; pins the spec correction)
    assert_eq!(box_sizing_for(None), TaffyBoxSizing::ContentBox);
}

#[test]
fn width_auto_with_side_borders_uses_border_box_for_taffy() {
    // Taffy 0.14 treats content-box width auto as content equal to the
    // containing width, overflowing side borders. CSS requires the border
    // box to fill for auto, which border-box gives taffy. This pins the
    // conic reference quadrants (auto width with 150px right border) to
    // 200 rather than 350. Fixed widths keep content-box.
    use raikiri_style::{build_rule_tree, cascade};
    fn box_sizing_for_width_and_borders(width: Option<&str>, borders: &str) -> TaffyBoxSizing {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let style = match width {
            Some(w) => format!("{w}; {borders}"),
            None => borders.to_string(),
        };
        let body = doc.append_element(Some(html), "body", Some(style.as_str()));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr).expect("apply Ok");
        bridged_style(&mut doc, &cr, body).box_sizing
    }
    assert_eq!(
        box_sizing_for_width_and_borders(None, "border-right: 150px solid red"),
        TaffyBoxSizing::BorderBox
    );
    assert_eq!(
        box_sizing_for_width_and_borders(Some("width: 200px"), "border-right: 150px solid red"),
        TaffyBoxSizing::ContentBox
    );
    assert_eq!(
        box_sizing_for_width_and_borders(None, "border-left: 50px solid black"),
        TaffyBoxSizing::BorderBox
    );
    assert_eq!(
        box_sizing_for_width_and_borders(None, "border-right: 150px solid red; padding-bottom: 0%"),
        TaffyBoxSizing::BorderBox
    );
    // Exercise the single-side shorthand CSS-wide and deferred paths so
    // rule expansion for the new left shorthand stays covered.
    let mut var_doc = Document::new();
    let var_html = var_doc.append_element(Some(0), "html", None::<&str>);
    let _ = var_doc.append_element(
        Some(var_html),
        "body",
        Some("border-left: inherit; border-left: var(--missing)"),
    );
    let var_rules = build_rule_tree(&var_doc);
    let var_cr = cascade(&var_doc, &var_rules).expect("cascade Ok");
    apply_computed_to_style(&mut var_doc, &var_cr).expect("apply Ok");
}

#[test]
fn page_layout_preserves_authored_body_width_from_bridge() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;width:100px"));
    let rules = raikiri_style::build_rule_tree(&doc);
    let cr = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout");
    assert_eq!(
        bridged_style(&mut doc, &cr, body).size.width,
        Dimension::length(100.0)
    );
    assert_eq!(doc.nodes[body].layout.size.width, 100.0);
}

#[test]
fn layout_single_page_bridges_display_flow_root() {
    // Standalone `display: flow-root` must reach taffy as its dedicated
    // block-formatting-context display value rather than falling through
    // to a plain block approximation.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flow_root = doc.append_element(Some(body), "div", Some("display:flow-root;width:200px"));
    let child = doc.append_element(Some(flow_root), "div", Some("height:20px;background:red"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    assert_eq!(
        bridged_display(&mut doc, &cr, flow_root),
        Display::FlowRoot,
        // cov:ignore: panic-message literal only executes on assertion failure.
        "bridge_display must map DisplayValue::FlowRoot to taffy::Display::FlowRoot"
    );
    assert!(doc.nodes[child].layout.size.height > 0.0);
}

#[test]
fn layout_single_page_bridges_display_flex_to_taffy_flexbox() {
    // Check by **geometry** that the display bridge delivers Display::Flex
    // to taffy::compute_flexbox_layout. Merely asserting style.display
    // shows that the bridge connects, not that compute_child_layout
    // in dispatch.rs actually invokes flex layout. Under the initial
    // CSS Flexbox Level 1 values
    // (`flex-direction: row`, `flex-wrap: nowrap`), two children with
    // explicit widths sit side by side on the main (x) axis and align on the cross (y) axis.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(Some(body), "div", Some("display:flex"));
    let child_a = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let child_b = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        bridged_display(&mut doc, &cr, flex_container),
        Display::Flex,
        "bridge_display must map DisplayValue::Flex to taffy::Display::Flex"
    );
    let a_loc = doc.nodes[child_a].layout.location;
    let b_loc = doc.nodes[child_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "row-direction flex items must share the same cross-axis (y) offset, got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "second flex item should sit 100px (first item's width) further along the main axis (x), got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
}

/// The alignments the bridge settles for a box with the computed values `cv`
/// in a container with the computed values `parent`.
fn settled_alignments(cv: &ComputedValues, parent: &ComputedValues) -> Settled {
    let mut settled = Settled::of(cv);
    let (align_self, justify_self) = settled::inherited_last_baseline(cv, parent);
    settled.align_self = align_self.or(settled.align_self);
    settled.justify_self = justify_self.or(settled.justify_self);
    settled
}

#[test]
fn self_alignment_to_taffy_maps_every_keyword() {
    // `self_alignment_to_taffy` backs `align-items` and the non-`auto` branch
    // of `align-self` — check every keyword→taffy constant mapping directly.
    for (value, expected) in [
        (SelfAlignmentValue::Normal, None),
        (SelfAlignmentValue::Stretch, Some(TaffyAlignItems::STRETCH)),
        (SelfAlignmentValue::Center, Some(TaffyAlignItems::CENTER)),
        (SelfAlignmentValue::Start, Some(TaffyAlignItems::START)),
        (SelfAlignmentValue::End, Some(TaffyAlignItems::END)),
        (
            SelfAlignmentValue::FlexStart,
            Some(TaffyAlignItems::FLEX_START),
        ),
        (SelfAlignmentValue::FlexEnd, Some(TaffyAlignItems::FLEX_END)),
        (
            SelfAlignmentValue::Baseline,
            Some(TaffyAlignItems::BASELINE),
        ),
        (
            SelfAlignmentValue::LastBaseline,
            Some(TaffyAlignItems::BASELINE),
        ),
    ] {
        assert_eq!(self_alignment_to_taffy(value), expected, "value: {value:?}");
    }
}

#[test]
fn settled_alignment_align_self_auto_is_none() {
    // The `AlignSelfValue::Auto` arm — the `None` mapping (CSS Box Alignment
    // 3 §6.2: falls back to the parent's `align-items`, delegated to taffy).
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Auto;
    let style = Settled::of(&cv);
    assert_eq!(style.align_self, None);
}

#[test]
fn settled_alignment_align_self_value_maps_through_self_alignment() {
    // The `AlignSelfValue::Value(v)` arm — distinct from the sibling test
    // above, which only pins the `Auto` arm. `self_alignment_to_taffy` itself
    // is pinned by `self_alignment_to_taffy_maps_every_keyword`, but that
    // doesn't exercise this arm's delegation to it.
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
    let style = Settled::of(&cv);
    assert_eq!(style.align_self, Some(TaffyAlignItems::CENTER));
}

#[test]
fn settled_alignment_last_baseline_falls_back_to_end_for_abspos() {
    // An absolutely positioned box takes no part in baseline alignment, so
    // `last baseline` falls back to `safe self-end` on both axes. A box in the
    // flow keeps aligning to the baseline.
    for (position, expected) in [
        (PositionValue::Absolute, TaffyAlignItems::SAFE_SELF_END),
        (PositionValue::Fixed, TaffyAlignItems::SAFE_SELF_END),
        (PositionValue::Static, TaffyAlignItems::BASELINE),
    ] {
        let label = format!("{position:?}");
        let mut cv = ComputedValues::initial();
        cv.position = position;
        cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::LastBaseline);
        cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::LastBaseline);
        let style = Settled::of(&cv);
        assert_eq!(style.align_self, Some(expected), "position: {label}");
        assert_eq!(style.justify_self, Some(expected), "position: {label}");
    }
}

#[test]
fn settled_alignment_baseline_falls_back_to_safe_start_for_abspos() {
    // An absolutely positioned box takes no part in baseline alignment, so
    // `baseline` keeps taffy's start placement with `safe` overflow alignment
    // on both axes. A box in the flow keeps aligning to the baseline.
    let safe_baseline = TaffyAlignItems {
        keyword: taffy::AlignItemsKeyword::Baseline,
        safety: taffy::AlignmentSafety::Safe,
    };
    for (position, expected) in [
        (PositionValue::Absolute, safe_baseline),
        (PositionValue::Fixed, safe_baseline),
        (PositionValue::Static, TaffyAlignItems::BASELINE),
    ] {
        let label = format!("{position:?}");
        let mut cv = ComputedValues::initial();
        cv.position = position;
        cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Baseline);
        cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::Baseline);
        let style = Settled::of(&cv);
        assert_eq!(style.align_self, Some(expected), "position: {label}");
        assert_eq!(style.justify_self, Some(expected), "position: {label}");
    }
}

#[test]
fn abspos_auto_alignment_takes_last_baseline_fallback_from_the_container() {
    let mut parent = ComputedValues::initial();
    parent.align_items = SelfAlignmentValue::LastBaseline;
    parent.justify_items = SelfAlignmentValue::LastBaseline;
    for (position, expected) in [
        (
            PositionValue::Absolute,
            Some(TaffyAlignItems::SAFE_SELF_END),
        ),
        (PositionValue::Fixed, Some(TaffyAlignItems::SAFE_SELF_END)),
        (PositionValue::Static, None),
    ] {
        let label = format!("{position:?}");
        let mut cv = ComputedValues::initial();
        cv.position = position;
        let style = settled_alignments(&cv, &parent);
        assert_eq!(style.align_self, expected, "position: {label}");
        assert_eq!(style.justify_self, expected, "position: {label}");
    }
}

#[test]
fn abspos_auto_alignment_takes_the_containers_last_baseline_only_for_the_static_position() {
    // `auto` borrows the container's `last baseline` only in an axis whose
    // insets are both `auto`; with one specified it behaves as `normal`.
    let mut parent = ComputedValues::initial();
    parent.align_items = SelfAlignmentValue::LastBaseline;
    parent.justify_items = SelfAlignmentValue::LastBaseline;
    let mut cv = ComputedValues::initial();
    cv.position = PositionValue::Absolute;
    cv.left = ComputedLengthPercentageOrAuto::Px(0.0);
    cv.bottom = ComputedLengthPercentageOrAuto::Px(0.0);
    let style = settled_alignments(&cv, &parent);
    assert_eq!(style.align_self, None);
    assert_eq!(style.justify_self, None);

    cv.left = ComputedLengthPercentageOrAuto::Auto;
    cv.bottom = ComputedLengthPercentageOrAuto::Auto;
    cv.top = ComputedLengthPercentageOrAuto::Px(0.0);
    let style = settled_alignments(&cv, &parent);
    assert_eq!(style.align_self, None);
    assert_eq!(style.justify_self, Some(TaffyAlignItems::SAFE_SELF_END));
}

#[test]
fn abspos_own_alignment_overrides_the_containers_last_baseline() {
    let mut parent = ComputedValues::initial();
    parent.align_items = SelfAlignmentValue::LastBaseline;
    let mut cv = ComputedValues::initial();
    cv.position = PositionValue::Absolute;
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
    let style = settled_alignments(&cv, &parent);
    assert_eq!(style.align_self, Some(TaffyAlignItems::CENTER));
    assert_eq!(style.justify_self, None);
}

#[test]
fn abspos_takes_last_baseline_from_a_display_contents_document_element() {
    use raikiri_style::{build_rule_tree, cascade};

    // A `display: contents` document element is blockified, so it still makes
    // the box its children's `auto` alignment borrows from; the document node
    // above it does not.
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Some("display:contents;align-items:last baseline;justify-items:last baseline"),
    );
    doc.append_element(Some(html), "body", None::<&str>);
    let child = doc.append_element(Some(html), "div", Some("position:absolute"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    apply_computed_to_style(&mut doc, &cr).expect("styles");

    let style = &bridged_style(&mut doc, &cr, child);
    assert_eq!(style.align_self, Some(TaffyAlignItems::SAFE_SELF_END));
    assert_eq!(style.justify_self, Some(TaffyAlignItems::SAFE_SELF_END));
}

#[test]
fn settled_alignment_align_self_normal_preserves_normal_instead_of_auto() {
    // Regression test: `align-self: normal` must
    // NOT collapse to the same `None` mapping as `align-self: auto`.
    // `auto` computes to the parent's `align-items` value (CSS Box
    // Alignment 3 §8.3), which taffy's `align_self: None` already
    // implements by inheriting the container's `align_items`. But
    // `normal` independently behaves as `stretch` in flex layout
    // regardless of the parent's `align-items` — mapping it to `None`
    // would incorrectly make it inherit the parent's value like `auto`
    // does. See the end-to-end behavioral check below.
    let mut cv = ComputedValues::initial();
    cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    let style = Settled::of(&cv);
    assert_eq!(style.align_self, Some(TaffyAlignItems::NORMAL));
}

#[test]
fn align_self_normal_stretches_even_when_parent_align_items_is_center() {
    // End-to-end check of the regression above: with the container's
    // `align-items: center`, a child with `align-self: auto` would
    // center (inheriting the parent's value per spec), but a child
    // with `align-self: normal` must independently stretch to fill the
    // cross axis — the two must NOT produce the same layout, even
    // though both map through `Option<AlignItems>`.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;height:100px;align-items:center"),
    );
    let normal_child = doc.append_element(
        Some(flex_container),
        "div",
        Some("width:50px;align-self:normal"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let child_height = doc.nodes[normal_child].layout.size.height;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (child_height - 100.0).abs() < 0.5,
        "align-self:normal must stretch to fill the 100px container cross-size regardless of the parent's align-items:center, got height={child_height}"
    );
}

#[test]
fn flex_direction_column_stacks_children_vertically() {
    // The `flex_direction` mapping must actually reach
    // taffy's `compute_flexbox_layout` — asserting `style.flex_direction
    // == Column` alone would only prove the assignment, not the integration,
    // since reading the field back off the taffy `Style` cannot
    // distinguish "assigned" from "used by the layout algorithm". With
    // `flex-direction: column`, the main axis flips to the block (y)
    // axis: 2 children with an explicit height must land at the same
    // x offset with y offsets 20px (the first child's height) apart.
    //
    // Also carries `gap:10px 30px` (row-gap column-gap) to independently
    // check `row-gap → taffy::Style::gap.height`, which the sibling
    // `gap_adds_space_between_flex_items` test cannot exercise — that
    // test's default row-direction container only puts `column-gap` on
    // the main axis. Here, `flex-direction: column` makes `row-gap` the
    // *main*-axis gap (`Size::main()` for a column container resolves to
    // `height`, per the gap mapping's `width: column_gap, height: row_gap`
    // mapping), so the two children's y-offset becomes 20px (first
    // child's height) + 10px (row-gap) = 30px — discriminating a dropped
    // row-gap (would stay at 20px) or a transposed bridge (would become
    // 20px + 30px = 50px) from the correct integration.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column;gap:10px 30px"),
    );
    let child_a = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let child_b = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let a_loc = doc.nodes[child_a].layout.location;
    let b_loc = doc.nodes[child_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.x - b_loc.x).abs() < 0.5,
        "column-direction flex items must share the same cross-axis (x) offset, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.y - a_loc.y - 30.0).abs() < 0.5,
        "second flex item should sit 30px (20px first item's height + 10px row-gap) further along the main axis (y), got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}

#[test]
fn flex_grow_absorbs_free_space() {
    // The `flex_grow` mapping reaching taffy's flexible-length
    // resolution algorithm (§9.7 "Resolving Flexible Lengths") — a
    // growable child must widen past its own basis to consume the free
    // space in the container, while a non-growable sibling stays at its
    // basis.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(Some(body), "div", Some("display:flex;width:500px"));
    let grower = doc.append_element(
        Some(flex_container),
        "div",
        Some("width:100px;height:20px;flex-grow:1"),
    );
    let fixed = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let grower_width = doc.nodes[grower].layout.size.width;
    let fixed_width = doc.nodes[fixed].layout.size.width;
    // Container is 500px, fixed sibling stays 100px, so the grower must
    // absorb the remaining 400px free space (500 - 100 = 400).
    //
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (grower_width - 400.0).abs() < 0.5,
        "flex-grow:1 item should absorb the container's free space, got width={grower_width}"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (fixed_width - 100.0).abs() < 0.5,
        "non-growing sibling should stay at its flex-basis width, got width={fixed_width}"
    );
}

#[test]
fn floated_box_with_explicit_width_is_positioned_at_the_containing_block_edge() {
    // Real float layout (taffy's `float_layout` feature, wired via
    // the float bridge + the `LayoutBlockContainer` impl of the session) —
    // a `float:left` box with an explicit width must be sized to
    // that width (not stretch to the container's full width like a
    // normal block child would) and must be positioned flush against
    // the containing block's start edge (CSS2 §9.5.1
    // <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>: "The
    // left outer edge of a left-floating box may not be to the left
    // of the left edge of its containing block").
    //
    // This does NOT exercise shrink-to-fit sizing (CSS2 §10.3.5
    // <https://www.w3.org/TR/CSS2/visudet.html#float-width>, which only
    // applies when 'width' computes to 'auto') — this fixture gives an
    // explicit width on purpose. Shrink-to-fit-width coverage is a
    // separate, currently-untested gap.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(Some(body), "div", Some("width:200px"));
    let float_child = doc.append_element(
        Some(container),
        "div",
        Some("float:left;width:60px;height:40px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let loc = doc.nodes[float_child].layout.location;
    let size = doc.nodes[float_child].layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (size.width - 60.0).abs() < 0.5,
        "explicit width must still be honored, got width={}",
        size.width
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        loc.x.abs() < 0.5,
        "left float must sit flush against the containing block's left edge, got x={}",
        loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        loc.y.abs() < 0.5,
        "first child's float must sit flush against the containing block's top edge, got y={}",
        loc.y
    );
}

#[test]
fn gap_adds_space_between_flex_items() {
    // The gap mapping puts the `gap` shorthand's 2 components onto taffy's
    // `Size<LengthPercentage>` as `Size { width: column_gap, height:
    // row_gap }`. The 2 components are deliberately asymmetric (10px
    // vs 30px) so that a transposed mapping (swapping row/column) would
    // fail this assertion instead of passing it unnoticed — in a
    // row-direction container (the default `flex-direction`), the
    // main axis (x) gap is `column-gap`, so the second item's x offset
    // must be the first item's width **plus 30px**, not 10px.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(Some(body), "div", Some("display:flex;gap:10px 30px"));
    // Source formatting whitespace is not an anonymous flex item. Keep
    // it in the DOM to exercise the same filtering used by HTML parsing.
    let _whitespace_before = doc.append_text(flex_container, "\n  ");
    let child_a = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let _whitespace_between = doc.append_text(flex_container, "\n  ");
    let child_b = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let _whitespace_after = doc.append_text(flex_container, "\n");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let a_loc = doc.nodes[child_a].layout.location;
    let b_loc = doc.nodes[child_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 130.0).abs() < 0.5,
        "second flex item should sit 130px (100px width + 30px column-gap) further along the main axis (x), got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
}

#[test]
fn justify_content_flex_end_pushes_children_to_container_end() {
    // `justify-content` reaching taffy's main-axis alignment (§9.5
    // "Main-Axis Alignment") — with
    // `justify-content: flex-end` in a 500px-wide row container, 2
    // 100px-wide children must land flush against the container's end
    // edge (x = 500 - 200 = 300 for the first child).
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let flex_container = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;width:500px;justify-content:flex-end"),
    );
    let child_a = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let child_b = doc.append_element(Some(flex_container), "div", Some("width:100px;height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let a_loc = doc.nodes[child_a].layout.location;
    let b_loc = doc.nodes[child_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.x - 300.0).abs() < 0.5,
        "first item should be pushed flush to the container's end edge (500 - 100 - 100 = 300), got a.x={}",
        a_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - 400.0).abs() < 0.5,
        "second item should sit immediately after the first (300 + 100 = 400), got b.x={}",
        b_loc.x
    );
}

#[test]
fn layout_single_page_bridges_display_grid_to_taffy_grid() {
    // Check that the display bridge delivers Display::Grid to
    // taffy::compute_grid_layout. Raikiri-style does not yet implement
    // grid-* properties such as `grid-template-columns` (outside this task's
    // scope, which only opens the entry point). An implicit single-track
    // grid is thus hard to distinguish from block layout. Here we only check
    // that the bridge selects Display::Grid and compute_grid_layout
    // returns finite boxes without crashing. Track-level checks belong
    // to a follow-up when grid-* properties are implemented.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let grid_container = doc.append_element(Some(body), "div", Some("display:grid"));
    let child_a = doc.append_element(Some(grid_container), "div", Some("width:100px;height:20px"));
    let child_b = doc.append_element(Some(grid_container), "div", Some("width:100px;height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        bridged_display(&mut doc, &cr, grid_container),
        Display::Grid,
        "bridge_display must map DisplayValue::Grid to taffy::Display::Grid"
    );
    for id in [child_a, child_b] {
        let size = doc.nodes[id].layout.size;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            size.width.is_finite()
                && size.height.is_finite()
                && size.width >= 0.0
                && size.height >= 0.0,
            "grid item layout must be finite and non-negative, got {:?}",
            size
        );
    }
}

#[test]
fn settled_justify_self_keeps_end_and_normal() {
    let mut cv = ComputedValues::initial();
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::End);
    let style = Settled::of(&cv);
    assert_eq!(style.justify_self, Some(TaffyAlignItems::END));

    // `justify-self: normal` behaves as `stretch` (CSS Box Alignment 3
    // §8.3), not as `None` (which would mean "inherit justify-items")
    // — same special-case as `align-self: normal`
    // (`self_alignment_or_auto_to_taffy` doc).
    let mut cv = ComputedValues::initial();
    cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
    let style = Settled::of(&cv);
    assert_eq!(style.justify_self, Some(TaffyAlignItems::NORMAL));
}

#[test]
fn grid_template_columns_actually_sizes_columns_through_taffy_grid_algorithm() {
    // The `grid_template_columns` mapping must actually reach
    // taffy's `compute_grid_layout` — asserting the `taffy::Style`
    // field alone would only prove the assignment, not the integration
    // (same rationale as `flex_direction_column_stacks_children_vertically`
    // above). A 2-column `100px 200px` grid with one child explicitly
    // placed in each column must position the second child 100px to
    // the right of the first.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let grid_container = doc.append_element(
        Some(body),
        "div",
        Some("display:grid;grid-template-columns:100px 200px"),
    );
    let cell_a = doc.append_element(
        Some(grid_container),
        "div",
        Some("grid-column:1;grid-row:1;height:20px"),
    );
    let cell_b = doc.append_element(
        Some(grid_container),
        "div",
        Some("grid-column:2;grid-row:1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let a_loc = doc.nodes[cell_a].layout.location;
    let b_loc = doc.nodes[cell_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "second grid cell should sit 100px (first column's width) to \
             the right of the first, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "both cells share row 1, so they must share the same y \
             offset, got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}

#[test]
fn grid_auto_flow_column_places_implicit_items_column_wise_through_taffy() {
    // The `grid_auto_flow` mapping reaching taffy's
    // auto-placement algorithm — with `grid-auto-flow: column` and no
    // explicit placement, 2 children must be auto-placed into
    // successive *columns* of the same row (not successive rows, the
    // `row` default), which the 100px `grid-auto-columns` track makes
    // observable as a 100px x-offset between them.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let _head = doc.append_element(Some(html), "head", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let grid_container = doc.append_element(
        Some(body),
        "div",
        Some("display:grid;grid-auto-flow:column;grid-auto-columns:100px"),
    );
    let cell_a = doc.append_element(Some(grid_container), "div", Some("height:20px"));
    let cell_b = doc.append_element(Some(grid_container), "div", Some("height:20px"));
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let a_loc = doc.nodes[cell_a].layout.location;
    let b_loc = doc.nodes[cell_b].layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
        "column-flow auto-placement should put the second item 100px \
             (grid-auto-columns) to the right of the first, got a.x={}, b.x={}",
        a_loc.x,
        b_loc.x
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (a_loc.y - b_loc.y).abs() < 0.5,
        "column-flow auto-placement should keep both items on row 1, \
             got a.y={}, b.y={}",
        a_loc.y,
        b_loc.y
    );
}

#[test]
fn multicol_style_keeps_both_terms_of_a_calc_gap() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;width:200px;column-gap:calc(10% + 5px)"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let style = multicol_style_from_computed(&cascade.computed[container]).expect("multicol style");
    assert!((style.gap - 5.0).abs() < 0.01, "{style:?}");
    assert_eq!(style.gap_percent, Some(10.0));
}

#[test]
fn multicol_style_resolves_normal_gap_to_one_em() {
    // CSS Multi-column Layout Module Level 1 section 5 Column Gaps and Rules
    // https://www.w3.org/TR/css-multicol-1/#column-gaps-and-rules
    // gives column-gap normal a used value of 1em on a multicol container.
    // The nested fragmentation seam carries that used gap in MulticolStyle,
    // so normal must resolve against the container font size here too, not
    // collapse to 0 like the flex and grid gap bridge does.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let default_container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;width:200px"),
    );
    let large_container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:2;width:200px;font-size:32px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let default_style =
        multicol_style_from_computed(&cascade.computed[default_container]).expect("multicol style");
    assert!(
        (default_style.gap - 16.0).abs() < 0.01,
        "default font-size 16px gives 16px gap, got {:?}",
        default_style
    );
    assert_eq!(default_style.gap_percent, None);
    let large_style =
        multicol_style_from_computed(&cascade.computed[large_container]).expect("multicol style");
    assert!(
        (large_style.gap - 32.0).abs() < 0.01,
        "font-size 32px gives 32px gap, got {:?}",
        large_style
    );
    assert_eq!(large_style.gap_percent, None);
}

#[test]
fn apply_computed_to_style_bridges_min_content_width_to_taffy() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let width = doc.append_element(Some(html), "div", Some("display:block;width:min-content"));
    let inline = doc.append_element(
        Some(html),
        "div",
        Some("display:block;inline-size:min-content"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");

    assert_eq!(
        bridged_style(&mut doc, &cascade, width).size.width,
        Dimension::min_content()
    );
    assert_eq!(
        bridged_style(&mut doc, &cascade, inline).size.width,
        Dimension::min_content()
    );
}

#[test]
fn apply_computed_to_style_bridges_max_and_fit_content_width_to_taffy() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let max = doc.append_element(Some(html), "div", Some("display:block;width:max-content"));
    let fit = doc.append_element(
        Some(html),
        "div",
        Some("display:block;inline-size:fit-content"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");

    assert_eq!(
        bridged_style(&mut doc, &cascade, max).size.width,
        Dimension::max_content()
    );
    assert_eq!(
        bridged_style(&mut doc, &cascade, fit).size.width,
        Dimension::fit_content()
    );
}

#[test]
fn apply_computed_to_style_bridges_the_fit_content_function_limit_to_taffy() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let widths: Vec<usize> = [
        "font-size:20px;width:fit-content(2em)",
        "width:fit-content(25%)",
        "inline-size:fit-content(calc(50% - 10px))",
        "width:fit-content(calc(10px + 5px))",
    ]
    .into_iter()
    .map(|css| doc.append_element(Some(html), "div", Some(&format!("display:block;{css}"))))
    .collect();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");
    let mut width = |node: usize| bridged_style(&mut doc, &cascade, node).size.width;

    assert_eq!(width(widths[0]), Dimension::fit_content_px(40.0));
    assert_eq!(width(widths[1]), Dimension::fit_content_percent(0.25));
    assert!(width(widths[2]).is_fit_content_calc());
    // A calc without a percentage is a plain length.
    assert_eq!(width(widths[3]), Dimension::fit_content_px(15.0));
    for node in widths {
        assert!(width(node).is_sizing_keyword());
    }
}

/// The Taffy size bridged for each `(parent css, child tag, child css)`.
fn bridged_sizes(cases: &[(&str, &str, &str)]) -> Vec<StyleSize<Dimension>> {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let children: Vec<usize> = cases
        .iter()
        .map(|&(parent_css, tag, child_css)| {
            let parent = doc.append_element(Some(html), "div", Some(parent_css));
            doc.append_element(Some(parent), tag, Some(child_css))
        })
        .collect();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");
    children
        .into_iter()
        .map(|child| bridged_style(&mut doc, &cascade, child).size)
        .collect()
}

/// The Taffy `width` bridged for each `(parent css, child tag, child css)`.
fn bridged_widths(cases: &[(&str, &str, &str)]) -> Vec<Dimension> {
    bridged_sizes(cases)
        .into_iter()
        .map(|size| size.width)
        .collect()
}

/// The keyword `min-*` and `max-*` sizes bridged for each
/// `(parent css, child tag, child css)`, with the Taffy min and max size.
fn bridged_content_limits(
    cases: &[(&str, &str, &str)],
) -> Vec<(
    crate::layout::dispatch::content_limits::ContentLimits,
    StyleSize<LengthPercentageAuto>,
    StyleSize<LengthPercentageAuto>,
)> {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let children: Vec<usize> = cases
        .iter()
        .map(|&(parent_css, tag, child_css)| {
            let parent = doc.append_element(Some(html), "div", Some(parent_css));
            doc.append_element(Some(parent), tag, Some(child_css))
        })
        .collect();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");
    children
        .into_iter()
        .map(|child| {
            let style = &bridged_style(&mut doc, &cascade, child);
            (
                doc.nodes[child].content_limits,
                style.min_size,
                style.max_size,
            )
        })
        .collect()
}

#[test]
fn a_block_in_normal_flow_records_its_keyword_min_and_max_sizes() {
    use crate::layout::dispatch::content_limits::ContentLimits;

    let css = "display:block;min-width:min-content;max-width:fit-content;\
               min-height:max-content;max-height:fit-content(10px)";
    for (parent, display) in [
        ("display:block", "block"),
        ("display:flow-root", "block"),
        ("display:block", "flow-root"),
        ("display:block", "list-item"),
    ] {
        let case = css.replace("display:block", &format!("display:{display}"));
        let [(limits, min, max)] = bridged_content_limits(&[(parent, "div", &case)])[..] else {
            panic!("one case");
        };
        assert_eq!(
            limits,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::max_content(),
                },
                max: StyleSize {
                    width: Dimension::fit_content(),
                    height: Dimension::fit_content_px(10.0),
                },
            },
            "{display} in {parent}"
        );
        // Taffy cannot hold a keyword and sees no constraint.
        assert_eq!(min, StyleSize::<LengthPercentageAuto>::auto());
        assert_eq!(max, StyleSize::<LengthPercentageAuto>::auto());
    }
}

#[test]
fn a_flex_or_grid_container_records_its_keyword_min_and_max_sizes() {
    use crate::layout::dispatch::content_limits::ContentLimits;

    let css = "min-width:min-content;max-width:fit-content;\
               min-height:max-content;max-height:fit-content(10px)";
    for display in ["flex", "grid"] {
        let child_css = format!("display:{display};{css}");
        let [(limits, min, max)] =
            bridged_content_limits(&[("display:block", "div", &child_css)])[..]
        else {
            panic!("one case");
        };
        assert_eq!(
            limits,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::max_content(),
                },
                max: StyleSize {
                    width: Dimension::fit_content(),
                    height: Dimension::fit_content_px(10.0),
                },
            },
            "display:{display}"
        );
        assert_eq!(min, StyleSize::<LengthPercentageAuto>::auto());
        assert_eq!(max, StyleSize::<LengthPercentageAuto>::auto());
    }
}

#[test]
fn a_flex_or_grid_item_records_its_keyword_min_and_max_sizes() {
    use crate::layout::dispatch::content_limits::ContentLimits;

    let css = "display:block;min-width:min-content;max-width:fit-content;\
               min-height:max-content;max-height:fit-content(10px)";
    for parent in ["display:flex", "display:grid"] {
        let [(limits, min, max)] = bridged_content_limits(&[(parent, "div", css)])[..] else {
            panic!("one case");
        };
        assert_eq!(
            limits,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::max_content(),
                },
                max: StyleSize {
                    width: Dimension::fit_content(),
                    height: Dimension::fit_content_px(10.0),
                },
            },
            "{parent}"
        );
        assert_eq!(min, StyleSize::<LengthPercentageAuto>::auto());
        assert_eq!(max, StyleSize::<LengthPercentageAuto>::auto());
    }
}

#[test]
fn a_keyword_limit_with_a_calc_is_left_for_taffy_to_resolve() {
    let [(limits, ..)] = bridged_content_limits(&[(
        "display:block",
        "div",
        "display:block;min-width:fit-content(calc(50% - 10px));max-height:fit-content(25%)",
    )])[..] else {
        panic!("one case");
    };
    assert!(limits.min.width.is_fit_content_calc());
    assert_eq!(limits.max.height, Dimension::fit_content_percent(0.25));
}

#[test]
fn a_size_that_is_no_keyword_is_no_content_limit() {
    use crate::layout::dispatch::content_limits::ContentLimits;

    let [(limits, min, max)] = bridged_content_limits(&[(
        "display:block",
        "div",
        "display:block;min-width:10px;max-width:50%;min-height:auto;max-height:none",
    )])[..] else {
        panic!("one case");
    };
    assert_eq!(limits, ContentLimits::NONE);
    assert_eq!(min.width, LengthPercentageAuto::length(10.0));
    assert_eq!(max.width, LengthPercentageAuto::percent(0.5));
}

#[test]
fn unsupported_generic_box_content_limits_record_none() {
    use crate::layout::dispatch::content_limits::ContentLimits;

    let keyword = "min-width:min-content;max-height:max-content";
    let block = format!("display:block;{keyword}");
    let cases: Vec<(&str, &str, String)> = vec![
        // An inline-level box.
        ("display:block", "span", format!("display:inline;{keyword}")),
        (
            "display:block",
            "div",
            format!("display:grid;{keyword};writing-mode:vertical-rl"),
        ),
        (
            "display:flex",
            "div",
            format!("display:grid;{keyword};writing-mode:vertical-rl"),
        ),
        // A multicol container and a box in its columns.
        ("display:block", "div", format!("{block};column-count:2")),
        ("display:block;column-count:2", "div", block.clone()),
        // A vertical box writes on physical axes different from its parent.
        (
            "display:block",
            "div",
            format!("{block};writing-mode:vertical-rl"),
        ),
    ];
    let borrowed: Vec<(&str, &str, &str)> = cases
        .iter()
        .map(|(parent, tag, css)| (*parent, *tag, css.as_str()))
        .collect();
    for (case, (limits, ..)) in borrowed.iter().zip(bridged_content_limits(&borrowed)) {
        assert_eq!(limits, ContentLimits::NONE, "{case:?}");
    }
}

#[test]
fn a_display_contents_parent_passes_its_own_container_to_the_content_limits() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let mut items = Vec::new();
    for container in ["display:flex", "display:block"] {
        let outer = doc.append_element(Some(html), "div", Some(container));
        let contents = doc.append_element(Some(outer), "div", Some("display:contents"));
        items.push(doc.append_element(
            Some(contents),
            "div",
            Some("display:block;min-width:max-content"),
        ));
    }
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");

    assert_eq!(
        doc.nodes[items[0]].content_limits.min.width,
        Dimension::max_content()
    );
    assert_eq!(
        doc.nodes[items[1]].content_limits.min.width,
        Dimension::max_content()
    );
}

#[test]
fn a_display_contents_parent_passes_its_own_container_to_the_width_keyword() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let mut items = Vec::new();
    for container in ["display:flex", "display:block"] {
        let outer = doc.append_element(Some(html), "div", Some(container));
        let contents = doc.append_element(Some(outer), "div", Some("display:contents"));
        items.push(doc.append_element(
            Some(contents),
            "div",
            Some("display:block;width:max-content;max-width:100px"),
        ));
    }
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");

    // The flex item is clamped, so its keyword stays out of Taffy's way; the
    // block is sized by the keyword and clamped afterwards.
    let mut width = |item: usize| bridged_style(&mut doc, &cascade, item).size.width;
    assert_eq!(width(items[0]), Dimension::auto());
    assert_eq!(width(items[1]), Dimension::max_content());
}

#[test]
fn a_box_a_sizing_keyword_does_not_size_keeps_an_auto_width() {
    for keyword in [
        "max-content",
        "fit-content",
        "fit-content(50px)",
        "fit-content(50%)",
    ] {
        let width = format!("display:block;width:{keyword}");
        let widths = bridged_widths(&[
            // A vertical flex or grid item takes its inline size from the
            // height, which the container sizes along its physical axes.
            (
                "display:flex",
                "div",
                &format!("{width};writing-mode:vertical-rl"),
            ),
            (
                "display:grid",
                "div",
                &format!("{width};writing-mode:vertical-rl"),
            ),
            // A flex or grid item with a minimum or maximum width is measured
            // by Taffy before the size is clamped.
            ("display:flex", "div", &format!("{width};max-width:100px")),
            ("display:flex", "div", &format!("{width};min-width:10px")),
            ("display:grid", "div", &format!("{width};max-width:50%")),
            // A replaced element takes its natural size, also when it is
            // absolutely positioned.
            ("display:block", "img", &width),
            ("display:block", "canvas", &width),
            (
                "display:block",
                "img",
                &format!("{width};position:absolute"),
            ),
        ]);
        assert!(
            widths.iter().all(|width| *width == Dimension::auto()),
            "{keyword}: {widths:?}"
        );
    }
}

#[test]
fn a_box_a_sizing_keyword_sizes_keeps_its_keyword_width() {
    let width = "display:block;width:max-content";
    let keyword = Dimension::max_content();
    assert_eq!(
        bridged_widths(&[
            (
                "display:block",
                "div",
                "display:block;width:fit-content(50px)"
            ),
            (
                "display:flow-root",
                "div",
                "display:block;width:fit-content(50%)"
            ),
            (
                "display:block",
                "div",
                "display:contents;width:fit-content(50px)"
            ),
        ]),
        [
            Dimension::fit_content_px(50.0),
            Dimension::fit_content_percent(0.5),
            Dimension::auto(),
        ],
    );
    assert_eq!(
        bridged_widths(&[
            ("display:block", "div", width),
            ("display:flow-root", "div", width),
            ("display:block", "div", "display:flex;width:max-content"),
            ("display:block", "div", "display:grid;width:max-content"),
            ("display:flex", "div", "position:absolute;width:max-content"),
            ("display:flex", "div", "position:fixed;width:max-content"),
            // A flex or grid item, also one that is a flex or grid container
            // itself.
            ("display:flex", "div", width),
            ("display:inline-flex", "div", width),
            ("display:grid", "div", width),
            ("display:flex", "div", "display:flex;width:max-content"),
            // A minimum width that is zero or `auto` clamps nothing.
            (
                "display:flex",
                "div",
                "display:block;width:max-content;min-width:0"
            ),
            (
                "display:grid",
                "div",
                "display:block;width:max-content;min-width:auto"
            ),
            (
                "display:flex",
                "div",
                "display:block;width:max-content;min-width:0%"
            ),
            (
                "display:grid",
                "div",
                "display:block;width:max-content;min-width:calc(0% + 0px)"
            ),
            // A multicol container is sized by its columns.
            (
                "display:block",
                "div",
                "display:block;column-width:10px;width:max-content;"
            ),
            (
                "display:flex",
                "div",
                "display:block;column-count:2;width:max-content;"
            ),
        ]),
        [keyword; 16],
    );
}

#[test]
fn a_size_that_is_zero_whatever_it_refers_to_is_a_zero_length() {
    use raikiri_style::property::CalcLengthPercentage;

    let calc =
        |percent, px| ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage { percent, px });
    for zero in [
        ComputedLengthPercentageOrAuto::Px(0.0),
        ComputedLengthPercentageOrAuto::Px(-0.0),
        ComputedLengthPercentageOrAuto::Percent(0.0),
        calc(0.0, 0.0),
    ] {
        assert!(is_zero_length(zero), "{zero:?}");
    }
    for other in [
        ComputedLengthPercentageOrAuto::Px(1.0),
        ComputedLengthPercentageOrAuto::Percent(5.0),
        calc(10.0, 0.0),
        calc(0.0, 4.0),
        ComputedLengthPercentageOrAuto::Auto,
        ComputedLengthPercentageOrAuto::MinContent,
    ] {
        assert!(!is_zero_length(other), "{other:?}");
    }
}

#[test]
fn a_vertical_box_takes_its_inline_size_keyword_from_the_height() {
    for keyword in ["max-content", "fit-content"] {
        let vertical = format!("display:block;writing-mode:vertical-rl;inline-size:{keyword}");
        let sizes = bridged_sizes(&[
            ("display:block", "div", &vertical),
            ("display:flex", "div", &vertical),
            ("display:block", "img", &vertical),
        ]);
        let expected = if keyword == "max-content" {
            Dimension::max_content()
        } else {
            Dimension::fit_content()
        };
        assert_eq!(sizes[0].height, expected, "{keyword} in a block");
        // A vertical flex item or a replaced element is not sized by the
        // keyword.
        assert_eq!(sizes[1].height, Dimension::auto(), "{keyword} in a flex");
        assert_eq!(sizes[2].height, Dimension::auto(), "{keyword} on an image");
    }
}

/// The Taffy `align-self` bridged for each `(parent css, child tag, child
/// css)`, in a child of a `display: contents` wrapper when `wrapped`.
fn bridged_align_selves(
    cases: &[(&str, &str, &str)],
    wrapped: bool,
) -> Vec<Option<taffy::AlignItems>> {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let children: Vec<usize> = cases
        .iter()
        .map(|&(parent_css, tag, child_css)| {
            let parent = doc.append_element(Some(html), "div", Some(parent_css));
            let parent = if wrapped {
                doc.append_element(Some(parent), "div", Some("display:contents"))
            } else {
                parent
            };
            doc.append_element(Some(parent), tag, Some(child_css))
        })
        .collect();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    apply_computed_to_style(&mut doc, &cascade).expect("bridge Ok");
    children
        .into_iter()
        .map(|child| bridged_style(&mut doc, &cascade, child).align_self)
        .collect()
}

#[test]
fn an_item_of_a_column_flex_container_with_a_keyword_width_is_aligned_at_the_start() {
    let start = Some(taffy::AlignItems::FLEX_START);
    let column = "display:flex;flex-direction:column";
    for keyword in [
        "min-content",
        "max-content",
        "fit-content",
        "fit-content(50px)",
        "fit-content(50%)",
    ] {
        let item = format!("display:block;width:{keyword}");
        for wrapped in [false, true] {
            assert_eq!(
                bridged_align_selves(
                    &[
                        (column, "div", &item),
                        ("display:flex;flex-direction:column-reverse", "div", &item),
                        ("display:inline-flex;flex-direction:column", "div", &item),
                        (
                            &format!("{column};align-items:stretch"),
                            "div",
                            &format!("{item};align-self:normal"),
                        ),
                        (column, "div", &format!("{item};align-self:stretch")),
                    ],
                    wrapped
                ),
                [start; 5],
                "{keyword} wrapped: {wrapped}"
            );
        }
    }
}

#[test]
fn an_item_that_is_not_stretched_keeps_its_alignment() {
    use taffy::AlignItems;

    let column = "display:flex;flex-direction:column";
    let keyword = "display:block;width:max-content";
    let cases = [
        // An alignment of its own, or the container's, that is no stretch.
        (column, "div", format!("{keyword};align-self:center")),
        (column, "div", format!("{keyword};align-self:baseline")),
        (
            "display:flex;flex-direction:column;align-items:center",
            "div",
            keyword.to_string(),
        ),
        // A width that is no keyword.
        (column, "div", "display:block;width:auto".to_string()),
        (column, "div", "display:block;width:100px".to_string()),
        // A container with another cross axis, or no flex layout.
        ("display:flex", "div", keyword.to_string()),
        ("display:grid", "div", keyword.to_string()),
        ("display:block", "div", keyword.to_string()),
        // A vertical item takes its sizes from the logical properties.
        (column, "div", format!("{keyword};writing-mode:vertical-rl")),
    ];
    let borrowed: Vec<(&str, &str, &str)> = cases
        .iter()
        .map(|(parent, tag, css)| (*parent, *tag, css.as_str()))
        .collect();
    assert_eq!(
        bridged_align_selves(&borrowed, false),
        [
            Some(AlignItems::CENTER),
            Some(AlignItems::BASELINE),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ]
    );
}

#[test]
fn an_item_with_a_keyword_height_is_aligned_at_the_start_of_the_block_axis() {
    // The cascade computes the keyword to `auto`, which a stretch would fill;
    // only the mark it leaves tells the item from one with an `auto` height.
    let start = Some(taffy::AlignItems::START);
    let flex_start = Some(taffy::AlignItems::FLEX_START);
    for keyword in ["min-content", "max-content", "fit-content"] {
        let item = format!("display:block;height:{keyword}");
        for wrapped in [false, true] {
            assert_eq!(
                bridged_align_selves(
                    &[
                        ("display:grid", "div", &item),
                        ("display:inline-grid", "div", &item),
                        ("display:flex", "div", &item),
                        ("display:flex;flex-direction:row-reverse", "div", &item),
                        ("display:inline-flex", "div", &item),
                        (
                            "display:flex;align-items:stretch",
                            "img",
                            &format!("{item};align-self:normal"),
                        ),
                        ("display:grid", "img", &format!("{item};align-self:stretch")),
                        (
                            "display:flex;align-items:center",
                            "div",
                            &format!("{item};align-self:stretch"),
                        ),
                    ],
                    wrapped
                ),
                [
                    start, start, flex_start, flex_start, flex_start, flex_start, start, flex_start
                ],
                "{keyword} wrapped: {wrapped}"
            );
        }
    }
}

#[test]
fn an_item_without_a_stretched_keyword_height_keeps_its_alignment() {
    use taffy::AlignItems;

    let keyword = "display:block;height:max-content";
    let cases = [
        // An alignment of its own, or the container's, that is no stretch.
        (
            "display:flex",
            "div",
            format!("{keyword};align-self:center"),
        ),
        (
            "display:grid",
            "div",
            format!("{keyword};align-self:baseline"),
        ),
        (
            "display:flex;align-items:center",
            "div",
            keyword.to_string(),
        ),
        // A height that is no keyword; `fit-content(<length>)` is stretched
        // like `auto`.
        (
            "display:flex",
            "div",
            "display:block;height:auto".to_string(),
        ),
        (
            "display:flex",
            "div",
            "display:block;height:100px".to_string(),
        ),
        (
            "display:grid",
            "div",
            "display:block;height:fit-content(50px)".to_string(),
        ),
        // A keyword width does not stop a stretch of the height.
        (
            "display:flex",
            "div",
            "display:block;width:max-content".to_string(),
        ),
        // A container whose cross axis is not the height, or no layout of items.
        (
            "display:flex;flex-direction:column",
            "div",
            keyword.to_string(),
        ),
        ("display:block", "div", keyword.to_string()),
        // An absolutely positioned box is no item of the alignment.
        (
            "display:grid",
            "div",
            format!("{keyword};position:absolute"),
        ),
        // Neither is a box that generates no box of its own.
        (
            "display:grid",
            "div",
            "display:contents;height:max-content".to_string(),
        ),
        (
            "display:flex",
            "div",
            "display:none;height:max-content".to_string(),
        ),
    ];
    let borrowed: Vec<(&str, &str, &str)> = cases
        .iter()
        .map(|(parent, tag, css)| (*parent, *tag, css.as_str()))
        .collect();
    assert_eq!(
        bridged_align_selves(&borrowed, false),
        [
            Some(AlignItems::CENTER),
            Some(AlignItems::BASELINE),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ]
    );
}

#[test]
fn an_item_inherits_the_keyword_height_of_its_container() {
    // `height: inherit` takes the keyword of the parent although the cascade
    // computes it to `auto`, so the item is held at the start like the parent.
    let start = Some(taffy::AlignItems::START);
    let flex_start = Some(taffy::AlignItems::FLEX_START);
    let inherit = "display:block;height:inherit";
    assert_eq!(
        bridged_align_selves(
            &[
                ("display:grid;height:min-content", "div", inherit),
                ("display:flex;height:max-content", "div", inherit),
                ("display:grid;height:auto", "div", inherit),
                ("display:flex;height:100px", "div", inherit),
            ],
            false
        ),
        [start, flex_start, None, None]
    );
}

#[test]
fn a_multicol_container_establishes_a_block_formatting_context() {
    // CSS Multi-column Layout 1, 2: the first child's top margin stays
    // inside the multicol container instead of collapsing with its own.
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0"));
    let multicol = doc.append_element(
        Some(body),
        "div",
        Some("display:block;column-count:1;width:200px"),
    );
    let child = doc.append_element(
        Some(multicol),
        "div",
        Some("display:block;height:20px;margin-top:15px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    assert_eq!(bridged_display(&mut doc, &cr, multicol), Display::FlowRoot);
    assert_eq!(doc.nodes[multicol].layout.location.y, 0.0);
    assert_eq!(doc.nodes[child].layout.location.y, 15.0);
    assert_eq!(doc.nodes[multicol].layout.size.height, 35.0);
}

#[test]
fn bridged_writing_modes_follow_live_parents_and_refresh_before_consumers() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", Some("display:block"));
    let early = doc.append_element(Some(body), "div", Some("display:block"));
    let text = doc.append_text(early, "x");
    let late = doc.append_element(
        Some(body),
        "div",
        Some("display:block;writing-mode:sideways-rl"),
    );
    let horizontal = doc.append_element(
        Some(late),
        "div",
        Some("display:block;writing-mode:horizontal-tb"),
    );
    let template = doc.append_element(Some(late), "template", None::<&str>);
    let fragment = doc.allocate_template_fragment_root(template);
    let inert = doc.append_text(fragment, "inert");
    doc.detach_from_parent(early);
    doc.attach_child(late, early);
    doc.mark_in_document_flags();
    let cr = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    apply_computed_to_style(&mut doc, &cr).unwrap();
    assert_eq!(doc.nodes[early].used_writing_mode, WritingMode::SidewaysRl);
    assert_eq!(doc.nodes[text].used_writing_mode, WritingMode::SidewaysRl);
    assert_eq!(
        doc.nodes[horizontal].used_writing_mode,
        WritingMode::HorizontalTb
    );
    assert!(doc.nodes[horizontal].under_vertical_writing);
    assert_eq!(
        doc.nodes[fragment].used_writing_mode,
        WritingMode::HorizontalTb
    );
    assert!(!doc.nodes[inert].under_vertical_writing);
    // Reusing declarations after reparenting must follow the live DOM,
    // as the previous nearest-authored ancestor walk did.
    doc.detach_from_parent(early);
    doc.attach_child(body, early);
    apply_computed_to_style(&mut doc, &cr).unwrap();
    assert_eq!(
        doc.nodes[early].used_writing_mode,
        WritingMode::HorizontalTb
    );
    assert!(!doc.nodes[text].under_vertical_writing);
    for mode in [
        "horizontal-tb",
        "vertical-rl",
        "vertical-lr",
        "sideways-rl",
        "sideways-lr",
        "horizontal-tb",
    ] {
        doc.set_element_inline_style(
            late,
            Some(format!("display:block;writing-mode:{mode}").into()),
        );
        let cr = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        apply_computed_to_style(&mut doc, &cr).unwrap();
        for id in 0..doc.nodes.len() {
            let expected = doc
                .inclusive_ancestors(id)
                .find_map(|id| doc.nodes[id].authored_writing_mode)
                .unwrap_or(WritingMode::HorizontalTb);
            let under = doc.inclusive_ancestors(id).any(|id| {
                doc.nodes[id]
                    .authored_writing_mode
                    .is_some_and(WritingMode::is_vertical)
            });
            assert_eq!(doc.nodes[id].used_writing_mode, expected);
            assert_eq!(doc.nodes[id].under_vertical_writing, under);
        }
    }
}

#[test]
fn changing_a_block_parent_to_flex_refreshes_inline_size_containment_before_classification() {
    use crate::layout::test_support::{page_box_800x600, with_ahem};
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/1 Ahem"));
    let parent = doc.append_element(Some(body), "div", Some("display:block"));
    let child = doc.append_element(Some(parent), "span", Some("contain:size"));
    doc.append_text(child, "XX");

    let first = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout_single_page(&mut doc, &first, page_box_800x600()).unwrap();
    assert_eq!(doc.nodes[parent].layout.size.height, 20.0);

    doc.set_element_inline_style(parent, Some("display:flex;align-items:start".into()));
    let second = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout_single_page(&mut doc, &second, page_box_800x600()).unwrap();
    assert_eq!(doc.nodes[child].layout.size, Size::ZERO);
}

#[test]
fn changing_a_table_parent_to_flex_or_grid_matches_fresh_size_containment() {
    use crate::layout::test_support::{page_box_800x600, with_ahem};
    use raikiri_style::{build_rule_tree, cascade};

    fn fixture(parent_css: &str) -> (Document, usize, usize) {
        let mut doc = Document::new();
        with_ahem(&mut doc);
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", Some("margin:0;font:20px/1 Ahem"));
        let parent = doc.append_element(Some(body), "div", Some(parent_css));
        let child = doc.append_element(Some(parent), "span", Some("contain:size"));
        doc.append_text(child, "XX");
        (doc, parent, child)
    }

    for final_css in [
        "display:flex;align-items:start",
        "display:grid;align-items:start;justify-items:start",
    ] {
        let (mut fresh, _, fresh_child) = fixture(final_css);
        let fresh_style = cascade(&fresh, &build_rule_tree(&fresh)).unwrap();
        crate::layout_single_page(&mut fresh, &fresh_style, page_box_800x600()).unwrap();
        let expected = fresh.nodes[fresh_child].layout.size;
        assert_eq!(expected, Size::ZERO);

        let (mut changed, parent, child) = fixture("display:table");
        let first = cascade(&changed, &build_rule_tree(&changed)).unwrap();
        crate::layout_single_page(&mut changed, &first, page_box_800x600()).unwrap();
        assert_eq!(
            changed.nodes[child].layout.size,
            Size {
                width: 40.0,
                height: 20.0
            }
        );
        changed.set_element_inline_style(parent, Some(final_css.into()));
        let second = cascade(&changed, &build_rule_tree(&changed)).unwrap();
        crate::layout_single_page(&mut changed, &second, page_box_800x600()).unwrap();
        assert_eq!(changed.nodes[child].layout.size, expected);
    }
}

#[test]
fn layout_values_of_reads_a_vertical_box_sizes_from_its_logical_properties() {
    let mut doc = Document::new();
    let vertical = doc.append_element(
        Some(0),
        "div",
        Some("display:block;writing-mode:vertical-rl;inline-size:50px;block-size:30px"),
    );
    let horizontal =
        doc.append_element(Some(0), "div", Some("display:block;width:20px;height:10px"));
    let cascade = crate::layout::test_support::cascaded(&doc);

    // `inline-size` runs down a vertical box, `block-size` across it.
    let values = layout_values_of(&doc, &cascade, vertical);
    assert_eq!(values.width, ComputedLengthPercentageOrAuto::Px(30.0));
    assert_eq!(values.height, ComputedLengthPercentageOrAuto::Px(50.0));
    let values = layout_values_of(&doc, &cascade, horizontal);
    assert_eq!(values.width, ComputedLengthPercentageOrAuto::Px(20.0));
    assert_eq!(values.height, ComputedLengthPercentageOrAuto::Px(10.0));
}

mod aspect_ratio_tests;
mod clamp_warning_tests;
mod content_limit_bridge_tests;
mod replaced_sizing_tests;
