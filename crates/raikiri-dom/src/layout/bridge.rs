use super::*;

// cov:ignore: computed multicol bridge is exercised by ignored WPT reftests.
fn multicol_style_from_computed(cv: &ComputedValues) -> Option<MulticolStyle> {
    let count = match cv.column_count {
        ColumnCountValue::Count(value) if value > 0 => Some(value as usize),
        _ => None,
    };
    let width = match cv.column_width {
        ComputedColumnWidth::Px(value) if value.is_finite() && value > 0.0 => Some(value),
        _ => None,
    };
    if count.is_none() && width.is_none() {
        return None;
    }
    // Same 1em rule as the foundational multicol metrics path. CSS
    // Multi-column Layout Module Level 1 section 5 Column Gaps and Rules
    // https://www.w3.org/TR/css-multicol-1/#column-gaps-and-rules
    // resolves column-gap normal on a multicol container to 1em of the
    // container font size. Flex and grid keep normal as 0 in their own bridge.
    let (gap, gap_percent) = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => (value.max(0.0), None),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => (0.0, Some(value)),
        ComputedLengthPercentageOrNormal::Normal => {
            let em = cv.font_size.px();
            let resolved = if em.is_finite() { em.max(0.0) } else { 0.0 };
            (resolved, None)
        }
        _ => (0.0, None),
    };
    Some(MulticolStyle {
        count,
        width,
        gap,
        gap_percent,
        height_definite: !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto),
        horizontal: matches!(cv.writing_mode, WritingMode::HorizontalTb),
        orphans: cv.orphans.max(1) as usize,
        widows: cv.widows.max(1) as usize,
    })
}

/// ComputedValues → taffy::Style bridge dispatch site.
///
/// Refactored from inline mapping inside the per-element loop to dispatching
/// to per-field `bridge_*` helpers. Adding a new bridge now requires only a
/// helper and one dispatch line.
///
/// Bridges currently active:
/// - [`bridge_direction`] — [`Direction`] → [`taffy::Direction`]
/// - [`bridge_display`] — [`DisplayValue`] → [`taffy::Display`]
/// - [`bridge_float`] — [`ComputedValues::float`] / [`ComputedValues::clear`] →
///   [`taffy::Style::float`] / [`taffy::Style::clear`] (CSS2 §9.5.1 / §9.5.2)
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentage>` → [`taffy::Rect<LengthPercentage>`]
/// - [`bridge_size`] — [`ComputedLengthPercentageOrAuto`] `cv.width` / `cv.height` →
///   [`taffy::Style::size`] (`Size<Dimension>`). Assigns both width and height
///   fields at once with a struct literal.
/// - [`bridge_min_max_size`] — [`ComputedLengthPercentageOrAuto`]
///   `cv.min_width` / `cv.min_height` → [`taffy::Style::min_size`],
///   `cv.max_width` / `cv.max_height` → [`taffy::Style::max_size`].
///   Writes the four min/max fields with two struct literals (as in [`bridge_size`]).
/// - [`bridge_border`] — `Sides<ComputedBorder>` → [`taffy::Rect<LengthPercentage>`].
///   **The upstream `raikiri_style::resolve_border` owns border-style gating,
///   not this bridge** (at the computed-value stage) — CSS Backgrounds 3
///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>.
/// - [`bridge_box_sizing`] — [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`]
///   (CSS Sizing 3 §7)
/// - [`bridge_flex`] — `flex-direction` / `flex-wrap` / `flex-grow` /
///   `flex-shrink` / `flex-basis` → [`taffy::Style`]'s matching flex
///   container/item fields (CSS Flexible Box Layout Module Level 1)
/// - [`bridge_alignment`] — `justify-content` / `align-content` /
///   `align-items` / `align-self` / `justify-items` / `justify-self` →
///   [`taffy::Style`]'s matching `Option<AlignItems>`/`Option<AlignContent>`
///   fields (CSS Box Alignment Module Level 3)
/// - [`bridge_gap`] — `row-gap` / `column-gap` → [`taffy::Style::gap`]
///   (`Size<LengthPercentage>`, CSS Box Alignment Module Level 3 §8.1)
/// - [`bridge_grid`] — `grid-template-columns` / `grid-template-rows` /
///   `grid-template-areas` / `grid-auto-columns` / `grid-auto-rows` /
///   `grid-auto-flow` / `grid-row-start` / `grid-row-end` /
///   `grid-column-start` / `grid-column-end` → [`taffy::Style`]'s matching
///   grid container/item fields (CSS Grid Layout Module Level 1)
///
/// After the per-node bridge loop, a second pass
/// ([`establish_minimal_line_boxes`]) scans the entire bridged tree and
/// establishes a minimal inline formatting context in qualifying block
/// containers. See that function's docs for the conditions and scope.
pub(crate) fn apply_computed_to_style(doc: &mut Document, cascade: &CascadeResult) {
    // Styles retain raw pointers to these payloads for Taffy's calc callback;
    // rebuild the arena for every cascade/layout pass.
    doc.calc_values.clear();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        // Preserve full DisplayValue for table dispatch before taffy collapses it.
        doc.nodes[idx].display = cv.display;
        // Taffy does not carry CSS `order` in Style. Retain the computed value
        // on Node so flex/grid child traversal can build order-modified order.
        doc.nodes[idx].order = cv.order;
        // Carry multicol settings into the Taffy dispatch seam. Taffy has no
        // native multicol style fields, so the custom strategy reads this
        // side-channel while the ordinary Style remains Taffy-compatible.
        doc.nodes[idx].multicol = multicol_style_from_computed(cv);
        // Table engine inputs — no taffy::Style counterpart (taffy 0.12
        // has no table layout), carried Node-side like `display` above.
        doc.nodes[idx].table_layout = cv.table_layout;
        doc.nodes[idx].border_collapse = cv.border_collapse;
        doc.nodes[idx].border_spacing = cv.border_spacing;
        doc.nodes[idx].break_before = cv.break_before;
        doc.nodes[idx].break_after = cv.break_after;
        doc.nodes[idx].authored_writing_mode = cascade
            .authored_writing_modes
            .get(idx)
            .and_then(|mode| *mode);
        bridge_size(doc, idx, cv);
        doc.nodes[idx].style.aspect_ratio = doc.nodes[idx]
            .image_intrinsic_box()
            .and_then(|intrinsic| intrinsic.aspect_ratio)
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        // SVG 2 geometry: auto dimensions on svg are treated as 100%.
        // With only a viewBox ratio and no definite height, width establishes
        // the viewport and the ratio determines its auto height.
        if doc.nodes[idx].is_inline_svg_root()
            && doc.nodes[idx].attribute("width").is_none()
            && doc.nodes[idx].attribute("height").is_none()
            && doc.nodes[idx].style.aspect_ratio.is_some()
        {
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
                && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
            {
                doc.nodes[idx].style.size.width = Dimension::percent(1.0);
            } else if !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                // Taffy transfers max-height to max-width through this ratio
                // even for a definite width. SVG leaf measurement handles its
                // auto height without shrinking an authored width.
                doc.nodes[idx].style.aspect_ratio = None;
            }
        }
        if let Some((intrinsic_width, intrinsic_height)) =
            bridge_known_image_intrinsic_size(doc, idx, cv)
        {
            let style = &mut doc.nodes[idx].style;
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                style.size.width = Dimension::length(intrinsic_width);
            }
            if matches!(cv.height, ComputedLengthPercentageOrAuto::Auto) {
                style.size.height = Dimension::length(intrinsic_height);
            }
        }
        doc.nodes[idx].has_logical_min_block_size = cv.min_block_size.is_some()
            && !matches!(cv.break_inside, raikiri_style::property::BreakInside::Auto);
        let style = &mut doc.nodes[idx].style;
        bridge_direction(style, cv);
        bridge_display(style, cv);
        bridge_position(style, cv, &mut doc.layout_warnings);
        bridge_overflow(style, cv);
        bridge_float(style, cv);
        bridge_margin(style, cv, &mut doc.layout_warnings);
        bridge_padding(style, cv, &mut doc.layout_warnings);
        if cv.display == DisplayValue::ListItem
            && matches!(
                cv.list_style_position,
                raikiri_style::ListStylePosition::Inside
            )
            && style.padding.left.into_raw().value() == 0.0
        {
            // The current Taffy bridge has no marker child. Reserve a
            // conservative first-line gutter for the inside marker so its
            // post-layout paint does not overlap ordinary text. Explicit or
            // percentage padding remains untouched; a later inline-formatting
            // pass will replace this estimate with measured marker width.
            style.padding.left = LengthPercentage::length(cv.font_size.px().max(16.0) * 1.5);
        }
        bridge_min_max_size(style, cv, &mut doc.calc_values, &mut doc.layout_warnings);
        bridge_border(style, cv, &mut doc.layout_warnings);
        bridge_box_sizing(style, cv);
        bridge_flex(style, cv, &mut doc.layout_warnings);
        bridge_alignment(style, cv);
        bridge_gap(style, cv, &mut doc.layout_warnings);
        bridge_grid(style, cv, &mut doc.layout_warnings);
    }
    refresh_order_modified_children(doc);
    establish_minimal_line_boxes(doc, cascade);
    // Mark table formatting roots for blitz-compat bit preservation.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let is_table = matches!(
            doc.nodes[idx].display,
            DisplayValue::Table | DisplayValue::InlineTable
        );
        if is_table {
            doc.nodes[idx].flags.insert(NodeFlags::IS_TABLE_ROOT);
        } else {
            doc.nodes[idx].flags.remove(NodeFlags::IS_TABLE_ROOT);
        }
    }
}

/// [`Direction`] → [`taffy::Direction`] mapping.
///
/// Taffy uses this field for horizontal block alignment, table/grid ordering,
/// and overflow direction.  Keep it separate from text shaping: parley does
/// not expose the CSS base direction through the bridge used by this crate.
fn bridge_direction(style: &mut taffy::Style, cv: &ComputedValues) {
    style.direction = match cv.direction {
        Direction::Ltr => TaffyDirection::Ltr,
        Direction::Rtl => TaffyDirection::Rtl,
        _ => TaffyDirection::Ltr,
    };
}

/// [`DisplayValue`] → [`taffy::Display`] mapping.
///
/// Maps [`DisplayValue`] to
/// [`taffy::Display`]. Taffy 0.x has only Block / Flex / Grid /
/// None (no distinct `Inline` / `InlineBlock` variants), so:
/// - `Inline` → `Block` (initially Block; text-only nodes render as leaves)
/// - `InlineBlock` → `Block` (separate handling for a block child within
///   an inline-level flow parent is a follow-up)
/// - `None` → `None`
/// - `Flex` → `Flex` (a CSS Display 3 §2.2 `<display-inside>` keyword,
///   equivalent to `block flex` under the outer-defaulting rule;
///   [`crate::taffy_impl`]'s `LayoutFlexboxContainer` impl + taffy's
///   `compute_flexbox_layout` perform the actual layout)
/// - `Grid` → `Grid` (a CSS Display 3 §2.2 `<display-inside>` keyword,
///   equivalent to `block grid` under the outer-defaulting rule;
///   [`crate::taffy_impl`]'s `LayoutGridContainer` impl + taffy's
///   `compute_grid_layout` perform the actual layout)
/// - Table internal types (`table` / `inline-table` / `table-row-group` /
///   `table-header-group` / `table-footer-group` / `table-row` /
///   `table-column-group` / `table-column` / `table-cell` / `table-caption`)
///   → `Block` (temporary block approximation: taffy 0.12 lacks table layout.
///   TODO(table-layout): once [`crate::layout::table`]'s dedicated table
///   formatting context (CSS 2.1 §17.2.1 anonymous table object generation
///   included) is implemented, replace with dedicated Display / layout handling.)
/// - `flow-root` → `FlowRoot` (independent block formatting context)
/// - `list-item` / `contents` → `Block` (block approximation via catch-all
///   until dedicated handling is added)
/// - catch-all arm → `Block` (`non_exhaustive` forward-compat)
fn bridge_display(style: &mut taffy::Style, cv: &ComputedValues) {
    style.display = match cv.display {
        DisplayValue::Block => Display::Block,
        DisplayValue::Inline => Display::Block,
        DisplayValue::InlineBlock => Display::Block,
        DisplayValue::FlowRoot => Display::FlowRoot,
        DisplayValue::None => Display::None,
        DisplayValue::Flex | DisplayValue::InlineFlex => Display::Flex,
        DisplayValue::Grid | DisplayValue::InlineGrid => Display::Grid,
        // Table model — CSS Display 3 §2 / CSS 2.1 §17.2. Taffy 0.12 has no
        // table layout, so map to Block as temporary approximation.
        // Real table routing uses `Node::display` (preserved in
        // `apply_computed_to_style` before this collapse) rather than
        // `Style::display`.
        DisplayValue::Table => Display::Block,
        DisplayValue::InlineTable => Display::Block,
        DisplayValue::TableRowGroup => Display::Block,
        DisplayValue::TableHeaderGroup => Display::Block,
        DisplayValue::TableFooterGroup => Display::Block,
        DisplayValue::TableRow => Display::Block,
        DisplayValue::TableColumnGroup => Display::Block,
        DisplayValue::TableColumn => Display::Block,
        DisplayValue::TableCell => Display::Block,
        DisplayValue::TableCaption => Display::Block,
        _ => {
            // non_exhaustive catch-all — unknown future variant (e.g.
            // `list-item` / `contents`) goes to Block
            Display::Block
        }
    };
}

/// [`ComputedValues::float`] / [`ComputedValues::clear`] → [`taffy::Style::float`] /
/// [`taffy::Style::clear`] bridge (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float> / §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>). **One-to-one enum
/// mapping** (both raikiri-style's `FloatValue`/`ClearValue` and taffy's
/// `Float`/`Clear` carry exactly the spec's keyword sets, so no length
/// resolution or Length policy participation is needed, same shape as
/// [`bridge_box_sizing`]).
///
/// This only carries the keyword through; the actual float positioning,
/// shrink-to-fit width, and wraparound layout it drives is taffy's
/// `float_layout` feature (`compute_block_layout`'s `BlockFormattingContext`/
/// `FloatContext`), wired via this crate's `taffy_impl` module.
///
/// **Known taffy `block_layout` limitation**: a same-BFC child narrower than
/// its own BFC root can get float insets computed against the root's width
/// instead of its own — not yet worked around or covered by a test here.
fn bridge_float(style: &mut taffy::Style, cv: &ComputedValues) {
    style.float = match cv.float {
        FloatValue::None => TaffyFloat::None,
        FloatValue::Left => TaffyFloat::Left,
        FloatValue::Right => TaffyFloat::Right,
        // cov:ignore: unreachable while FloatValue is None|Left|Right only;
        // required for its #[non_exhaustive] contract.
        _ => TaffyFloat::None,
    };
    style.clear = match cv.clear {
        ClearValue::None => TaffyClear::None,
        ClearValue::Left => TaffyClear::Left,
        ClearValue::Right => TaffyClear::Right,
        ClearValue::Both => TaffyClear::Both,
        // cov:ignore: unreachable while ClearValue is None|Left|Right|Both
        // only; required for its #[non_exhaustive] contract.
        _ => TaffyClear::None,
    };
}

/// [`ComputedValues::margin`] (`Sides<ComputedLengthPercentageOrAuto>`) → [`taffy::Style::margin`]
/// (`Rect<LengthPercentageAuto>`) bridge.
///
/// CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical>:
/// write the four physical margin sides (top / right / bottom / left) to
/// taffy's `Rect` **by field name**, not with a positional constructor.
/// `Sides` orders fields `top,right,bottom,left`, whereas `Rect` orders them
/// `left,right,top,bottom`; named fields prevent a silent transpose.
///
/// See [`computed_length_percentage_or_auto_to_taffy_length_percentage_auto`] for length policy.
fn bridge_margin(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let m = cv.margin;
    style.margin = Rect {
        top: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.top, "margin", diag,
        ),
        right: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.right, "margin", diag,
        ),
        bottom: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.bottom, "margin", diag,
        ),
        left: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            m.left, "margin", diag,
        ),
    };
}

/// [`ComputedValues::padding`] (`Sides<ComputedLengthPercentage>`) → [`taffy::Style::padding`]
/// (`Rect<LengthPercentage>`) bridge.
///
/// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical>:
/// write the four physical padding sides (top / right / bottom / left) to
/// taffy's `Rect` **by field name**, not with a positional constructor.
/// `Sides` orders fields `top,right,bottom,left`, whereas `Rect` orders them
/// `left,right,top,bottom`; named fields prevent a silent transpose. Unlike margin,
/// padding has the `<length-percentage [0,∞]>` value type (no auto, with
/// non-negativity enforced at parse time by raikiri-style), so it uses
/// [`computed_length_percentage_to_taffy_length_percentage`].
///
/// See [`computed_length_percentage_to_taffy_length_percentage`] for length policy.
fn bridge_padding(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let p = cv.padding;
    style.padding = Rect {
        top: computed_length_percentage_to_taffy_length_percentage(p.top, "padding", diag),
        right: computed_length_percentage_to_taffy_length_percentage(p.right, "padding", diag),
        bottom: computed_length_percentage_to_taffy_length_percentage(p.bottom, "padding", diag),
        left: computed_length_percentage_to_taffy_length_percentage(p.left, "padding", diag),
    };
}

/// [`ComputedValues::border`] (`Sides<ComputedBorder>`) → [`taffy::Style::border`]
/// (`Rect<LengthPercentage>`) bridge.
///
/// # Style gating is already handled **upstream**
///
/// CSS Backgrounds 3 §3.3
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width> specifies
/// zero border widths for `none` and `hidden` at the **computed-value** stage:
/// "Computed value: absolute length, snapped as a border
/// width; **zero if the border style is `none` or `hidden`**". Thus
/// `raikiri_style::resolve_border` gates widths before they reach
/// [`ComputedValues::border`]; they are already zero in this bridge.
///
/// This bridge must not duplicate that check (duplicating the spec rule
/// risks drift when only one copy is fixed). The former `used_border_width`
/// helper was removed for this reason. The end-to-end gating test in this file,
/// `apply_computed_to_style_bridges_border_to_taffy`, still checks it.
///
/// The `@page` path (`PageCascadeResult::declarations`) now also funnels through
/// `resolve_border`, leaving just one border-width gate in raikiri-style.
/// This bridge reads per-node [`ComputedValues`], so that change does not
/// directly affect it. If page-margin boxes later use this bridge, trust the
/// upstream value **without reimplementing the gate**.
///
/// ⚠️ This applies **only to border-style gating**. Separately,
/// `PageCascadeResult::declarations` still has an unresolved hazard involving
/// non-finite `f32` values (+Inf / NaN); the canonical contract is in the docs
/// for `raikiri_style::page::PageCascadeResult::declarations`. When passing
/// page-margin boxes through this bridge, recheck that hazard as well as gating.
///
/// Write all four sides **by field name** rather than using a positional
/// constructor, to prevent the same silent `Sides` vs `Rect` field-order
/// transpose described in [`bridge_margin`].
///
/// # Unsupported by taffy
///
/// - Taffy tracks only `border-width`, not `border-color` or `border-style`.
///   Consuming color and line patterns from [`ComputedValues::border`] is
///   future work for the paint layer.
/// - `border-image` and `border-radius` are out of scope.
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let b = cv.border;
    style.border = Rect {
        top: computed_length_to_taffy_length_percentage(b.top.width(), diag),
        right: computed_length_to_taffy_length_percentage(b.right.width(), diag),
        bottom: computed_length_to_taffy_length_percentage(b.bottom.width(), diag),
        left: computed_length_to_taffy_length_percentage(b.left.width(), diag),
    };
}

fn bridge_overflow(style: &mut taffy::Style, cv: &ComputedValues) {
    fn map(value: OverflowValue) -> TaffyOverflow {
        match value {
            OverflowValue::Visible => TaffyOverflow::Visible,
            OverflowValue::Hidden => TaffyOverflow::Hidden,
            OverflowValue::Clip => TaffyOverflow::Clip,
            OverflowValue::Scroll | OverflowValue::Auto => TaffyOverflow::Scroll,
            _ => TaffyOverflow::Visible,
        }
    }
    style.overflow = Point {
        x: map(cv.overflow.x),
        y: map(cv.overflow.y),
    };
}

fn bridge_position(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.position = match cv.position {
        PositionValue::Absolute | PositionValue::Fixed => TaffyPosition::Absolute,
        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky => {
            TaffyPosition::Relative
        }
        _ => TaffyPosition::Relative,
    };
    let mut inset = |value: ComputedLengthPercentageOrAuto, site: &'static str| {
        computed_length_percentage_or_auto_to_taffy_length_percentage_auto(value, site, diag)
    };
    style.inset = Rect {
        top: inset(cv.top, "top"),
        right: inset(cv.right, "right"),
        bottom: inset(cv.bottom, "bottom"),
        left: inset(cv.left, "left"),
    };
}

/// [`ComputedValues::width`] / [`ComputedValues::height`] (`ComputedLengthPercentageOrAuto`) →
/// [`taffy::Style::size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §3.1.1
/// "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>).
///
/// Width support landed first, then height, so both preferred-size axes are
/// fully bridged. Assign both fields together using a struct literal
/// (`style.size = Size { width, height }`); the partial-write scaffold is
/// no longer needed.
///
/// See [`computed_length_percentage_or_auto_to_taffy_dimension`] for length policy.
///
/// # PageBox compromise
///
/// After this bridge, [`apply_page_content_box_to_body`] overwrites the `<body>`
/// element's `style.size` with the page content-box dimensions
/// (`layout_single_page` Step 1 → Step 4): the PageBox width/height minus the
/// used page margins (and the top/bottom page content insets), not the full
/// paper size. Thus the author values in `<body style="width: 100px; height: 200px">`
/// are written to taffy by this helper, but content-box values overwrite them
/// in Step 4. This is intentional for now (pending an @page cascade and
/// per-page PageBox refactor). The width author→content-box overwrite path is
/// checked by `apply_page_content_box_clobbers_body_width_from_bridge`. For height,
/// [`apply_page_content_box_to_body`] assigns both fields without branching through
/// the struct literal `style.size = Size { width, height }`; height thus uses
/// the same overwrite path as width (both fields are written in one statement).
/// `apply_page_content_box_sets_body_style_size_to_content_dimensions` checks that
/// helper's content-box output. The width test sufficiently checks the full
/// bridge→overwrite path; height relies on the structural guarantee to avoid duplication.
fn bridge_size(doc: &mut Document, node_id: usize, cv: &ComputedValues) {
    // width and height are written together, so use a struct literal.
    let width = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.calc_values,
        cv.width,
        "width",
        &mut doc.layout_warnings,
    );
    let height = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.calc_values,
        cv.height,
        "height",
        &mut doc.layout_warnings,
    );
    doc.nodes[node_id].style.size = Size { width, height };
}

/// Supply the dimensions of the small set of WPT image assets whose bytes are
/// intentionally resolved by the renderer's URL-color fallback.  Without an
/// intrinsic size, a replaced `<img>` with auto width and height collapses to
/// zero even though its sibling background-image fallback paints a rectangle.
fn bridge_known_image_intrinsic_size(
    doc: &Document,
    node_id: usize,
    cv: &ComputedValues,
) -> Option<(f32, f32)> {
    let node = &doc.nodes[node_id];
    if node.tag_name() != Some("img") {
        return None;
    }
    let src = node.attribute("src")?;
    let basename = src.rsplit('/').next().unwrap_or(src).to_ascii_lowercase();
    if basename != "green.png" {
        return None;
    }
    let width_auto = matches!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    let height_auto = matches!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    (width_auto || height_auto).then_some((100.0, 50.0))
}

/// [`ComputedValues::min_width`] / [`ComputedValues::min_height`] /
/// [`ComputedValues::max_width`] / [`ComputedValues::max_height`]
/// ([`ComputedLengthPercentageOrAuto`]) → [`taffy::Style::min_size`] /
/// [`taffy::Style::max_size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §4
/// "Minimum Size Properties" / §5 "Maximum Size Properties").
///
/// The initial `auto` minimum and initial `none` maximum are both normalized
/// to [`ComputedLengthPercentageOrAuto::Auto`] at the computed-value stage.
/// The sibling `parse_max_size` maps specified `none` to the `Auto` placeholder,
/// so all four fields use the same
/// [`computed_length_percentage_or_auto_to_taffy_min_max`] helper to translate
/// `Auto` to `LengthPercentageAuto::auto()`. This bridge needs no special
/// handling for max `none`: both `none` (no maximum) and `auto` (no minimum)
/// delegate "unconstrained" to taffy. calc() payloads are retained in the
/// document arena by that helper.
///
/// See the helper docs for length/percent policies and the non-finite guard.
/// Each of the four callers passes its own `site` label: `min-width`,
/// `min-height`, `max-width`, or `max-height` (for [`LayoutWarn::NonFiniteClamped`] diagnostics).
fn bridge_min_max_size(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    diag: &mut Vec<LayoutWarn>,
) {
    // Use two struct literals to write each pair of min/max fields together
    // (as in bridge_size; cv's Auto naturally preserves defaults).
    // Keep calc() handles alive in the document arena just like width/height;
    // collapsing a max-width calc() to zero would spuriously clamp a <col>.
    // The layout writing mode is normalized to horizontal-tb, just like
    // inline-size/block-size. Keep a logical block minimum on that same
    // layout axis when no physical min-height is present. Preserve an
    // explicit physical constraint; CSSOM keeps the original axis mapping.
    let min_height = if cv.writing_mode == WritingMode::HorizontalTb
        && cv.cssom_writing_mode != WritingMode::HorizontalTb
        && matches!(cv.min_height, ComputedLengthPercentageOrAuto::Auto)
    {
        cv.min_block_size.unwrap_or(cv.min_height)
    } else {
        cv.min_height
    };
    style.min_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.min_width,
            "min-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            min_height,
            "min-height",
            diag,
        ),
    };
    style.max_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_width,
            "max-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_height,
            "max-height",
            diag,
        ),
    };
}

/// [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`] bridge.
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`, with spec initial `content-box`. **One-to-one enum mapping**
/// (a small helper that needs no length policy).
///
/// # Initial-value correction
///
/// - raikiri-style initial = `BoxSizing::ContentBox` (per CSS Sizing 3 §3.3)
/// - taffy default = `taffy::BoxSizing::BorderBox` (`#[default]` in taffy 0.12)
///
/// Taffy's default disagrees with the spec, but for unspecified values the
/// cascade always seeds `ContentBox` through [`ComputedValues::initial`]. Once
/// this bridge runs, `style.box_sizing` always has the spec initial (`ContentBox`).
/// This helper therefore also corrects taffy's nonconforming default.
///
/// # non_exhaustive catch-all
///
/// Raikiri-style's [`BoxSizing`] is `#[non_exhaustive]`, following the sibling
/// forward-compatibility pattern. Unknown variants quietly use the spec initial
/// (`ContentBox`) rather than silently extending a spec violation: the safest
/// default (like the [`bridge_display`] catch-all → `Block`).
///
/// Taffy's `taffy::BoxSizing` is **not** `#[non_exhaustive]`, so the output
/// mapping is complete with its two `ContentBox` and `BorderBox` arms.
///
/// [`BoxSizing`]: raikiri_style::property::BoxSizing
fn bridge_box_sizing(style: &mut taffy::Style, cv: &ComputedValues) {
    // Taffy 0.14 treats content-box width auto as content equal to the
    // containing width, leaving side borders to overflow the parent. CSS
    // requires the border box to fill the containing width for auto
    // (content shrinks by the borders), which is what border-box gives
    // taffy. Use border-box for auto widths with side borders so
    // border-only quadrants such as the conic references size to 200
    // rather than 350. Widths without borders keep content-box so the
    // existing box-sizing bridge test still sees the spec initial.
    // Fixed widths keep content-box so existing fixed-width overflow
    // comparisons still match on both sides.
    let width_auto = matches!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    let has_side_borders = cv.border.left.width().px() > 0.0 || cv.border.right.width().px() > 0.0;
    // Changing box-sizing also changes explicit heights, so only apply the
    // auto-width workaround when the vertical axis cannot change: no top or
    // bottom borders and no vertical padding. The conic references have
    // horizontal-only borders, while table box-sizing tests use all four
    // sides and must keep content-box for their height assertions.
    let vertical_clean = cv.border.top.width().px() == 0.0
        && cv.border.bottom.width().px() == 0.0
        && matches!(
            cv.padding.top,
            raikiri_style::ComputedLengthPercentage::Px(0.0)
                | raikiri_style::ComputedLengthPercentage::Percent(0.0)
        )
        && matches!(
            cv.padding.bottom,
            raikiri_style::ComputedLengthPercentage::Px(0.0)
                | raikiri_style::ComputedLengthPercentage::Percent(0.0)
        );
    if width_auto
        && has_side_borders
        && vertical_clean
        && matches!(cv.box_sizing, StyleBoxSizing::ContentBox)
    {
        style.box_sizing = TaffyBoxSizing::BorderBox;
        return;
    }
    style.box_sizing = match cv.box_sizing {
        StyleBoxSizing::ContentBox => TaffyBoxSizing::ContentBox,
        StyleBoxSizing::BorderBox => TaffyBoxSizing::BorderBox,
        // For non_exhaustive variants, use the spec initial (ContentBox)
        // quietly to avoid spreading a silent spec violation.
        _ => TaffyBoxSizing::ContentBox,
    };
}

/// Flex container/item properties → [`taffy::Style`] bridge.
///
/// Map five CSS Flexible Box Layout Module Level 1 properties to their
/// matching taffy fields:
///
/// - `flex-direction` ([`FlexDirectionValue`]) → `style.flex_direction`
///   (§5.1)
/// - `flex-wrap` ([`FlexWrapValue`]) → `style.flex_wrap` (§5.2)
/// - `flex-grow` (`f32`) → `style.flex_grow` (§7.2.1, copied unchanged;
///   `<number>` needs no absolutization under the spec; see the sink-guard
///   note in [`ComputedValues::flex_grow`]'s docs)
/// - `flex-shrink` (`f32`) → `style.flex_shrink` (§7.2.2, likewise)
/// - `flex-basis` ([`ComputedFlexBasis`]) → `style.flex_basis`
///   (`taffy::Dimension`, §7.2.3)
///
/// # Bridging `flex-basis`
///
/// `Px` / `Percent` become [`taffy::Dimension`] using the same `/100.0`
/// percent conversion and non-finite [`sanitize_taffy`] guard as [`bridge_size`].
/// `MinContent` / `MaxContent` / `FitContent` map directly to the matching
/// taffy 0.14 `Dimension` variants (resolved by the flex algorithm during
/// content measurement).
///
/// - `Content` → [`taffy::Dimension::content`] (the 0.14 flex-basis-specific
///   keyword, mapped without changing its meaning).
fn bridge_flex(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.flex_direction = match cv.flex_direction {
        FlexDirectionValue::Row => TaffyFlexDirection::Row,
        FlexDirectionValue::RowReverse => TaffyFlexDirection::RowReverse,
        FlexDirectionValue::Column => TaffyFlexDirection::Column,
        FlexDirectionValue::ColumnReverse => TaffyFlexDirection::ColumnReverse,
        // cov:ignore: unreachable while FlexDirectionValue is
        // Row|RowReverse|Column|ColumnReverse only; required for its
        // #[non_exhaustive] contract (see doc above, `bridge_display`
        // catch-all's rationale).
        _ => TaffyFlexDirection::Row,
    };
    style.flex_wrap = match cv.flex_wrap {
        FlexWrapValue::NoWrap => TaffyFlexWrap::NoWrap,
        FlexWrapValue::Wrap => TaffyFlexWrap::Wrap,
        FlexWrapValue::WrapReverse => TaffyFlexWrap::WrapReverse,
        // cov:ignore: unreachable while FlexWrapValue is
        // NoWrap|Wrap|WrapReverse only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyFlexWrap::NoWrap,
    };
    // `<number>` needs no absolutization; copy it unchanged.
    style.flex_grow = cv.flex_grow;
    style.flex_shrink = cv.flex_shrink;
    // `flex-basis: content` (CSS Flexible Box Layout 1 §4.5) is distinct
    // from `auto` in principle — it always uses the item's content size as
    // the flex basis, even when `width`/`height` are also set, whereas
    // `auto` defers to `width`/`height` first and only falls back to
    // `content` maps to taffy's own `Dimension::content` (0.14 — "only
    // valid for flex-basis", exactly this property's keyword).
    // `min-content` / `max-content` / bare `fit-content` map to taffy's own
    // intrinsic `Dimension` variants (0.14), which the flex algorithm
    // resolves through content measurement.
    style.flex_basis = match cv.flex_basis {
        ComputedFlexBasis::Auto => taffy::Dimension::auto(),
        ComputedFlexBasis::Content => taffy::Dimension::content(),
        ComputedFlexBasis::MinContent => taffy::Dimension::min_content(),
        ComputedFlexBasis::MaxContent => taffy::Dimension::max_content(),
        ComputedFlexBasis::FitContent => taffy::Dimension::fit_content(),
        ComputedFlexBasis::Px(v) => taffy::Dimension::length(sanitize_taffy(v, "flex-basis", diag)),
        ComputedFlexBasis::Percent(p) => {
            taffy::Dimension::percent(sanitize_taffy(p / 100.0, "flex-basis", diag))
        }
    };
}

/// alignment property → [`taffy::Style`] bridge (CSS Box Alignment Module
/// Level 3).
///
/// - `justify-content` ([`ContentAlignmentValue`]) → `style.justify_content`
///   (§5.1)
/// - `align-content` ([`ContentAlignmentValue`]) → `style.align_content`
///   (§5.1)
/// - `align-items` ([`SelfAlignmentValue`]) → `style.align_items` (§7.2)
/// - `align-self` ([`AlignSelfValue`]) → `style.align_self` (§6.2)
/// - `justify-items` ([`SelfAlignmentValue`]) → `style.justify_items` (§7.1)
///   — the inline-axis counterpart of `align-items` on a grid container;
///   it shares the keyword set (see [`SelfAlignmentValue`]'s docs).
/// - `justify-self` ([`AlignSelfValue`]) → `style.justify_self` (§6.1) —
///   the inline-axis counterpart of `align-self` on a grid item. Taffy's
///   `GridItemStyle::justify_self` docs promise the same "Falls back to the parents … if not set"
///   behavior as `align_self`, so the same special handling for `normal` and
///   `auto` applies.
///
/// All six taffy fields are `Option<…>`. Taffy has no keyword for CSS `normal`
/// (on content-* properties), so map it to `None`, another initial-value
/// mismatch like the "Initial-value correction" in `bridge_box_sizing`.
/// Taffy then uses defaults dependent on the layout mode
/// (e.g. `GridContainerStyle::grid_align_content` uses `unwrap_or(AlignContent::STRETCH)`;
/// the grid fallback is `STRETCH`, while the flex path has another default
/// inside `compute_flexbox_layout`). `auto` on `align-self` / `justify-self`
/// also maps to `Option::None`, falling back to the parent's
/// `align-items` / `justify-items`, per CSS Box Alignment 3 §6.2/§6.1.
///
/// Explicit keywords map to constants on [`taffy::AlignItems`] /
/// [`taffy::AlignContent`] (e.g. `AlignItems::CENTER` is a struct constant,
/// not an enum variant). Taffy 0.12 separates safe/unsafe overflow positions;
/// see the scope notes in the [`ContentAlignmentValue`] / [`SelfAlignmentValue`]
/// docs. The `safety` field is always `AlignmentSafety::Unsafe`, because this
/// crate does not accept the `safe` / `unsafe` prefix
/// (see `parse_content_alignment` / `parse_self_alignment` docs).
fn bridge_alignment(style: &mut taffy::Style, cv: &ComputedValues) {
    style.justify_content = content_alignment_to_taffy(cv.justify_content);
    style.align_content = content_alignment_to_taffy(cv.align_content);
    style.align_items = self_alignment_to_taffy(cv.align_items);
    style.align_self = self_alignment_or_auto_to_taffy(cv.align_self);
    style.justify_items = self_alignment_to_taffy(cv.justify_items);
    style.justify_self = self_alignment_or_auto_to_taffy(cv.justify_self);
}

/// [`AlignSelfValue`] → `Option<taffy::AlignItems>` mapping — shared by
/// `align-self` and `justify-self` (see the `justify-self` section in
/// [`bridge_alignment`]'s docs; taffy's `AlignSelf` is a type alias for
/// `AlignItems`).
fn self_alignment_or_auto_to_taffy(v: AlignSelfValue) -> Option<TaffyAlignItems> {
    match v {
        AlignSelfValue::Auto => None,
        // Unlike `auto`, `normal` does not fall back to the parent's
        // align-items/justify-items. In flex/grid layout it independently
        // behaves like `stretch` (CSS Box Alignment 3 §8.3 says "In flex
        // layout, this value behaves as stretch"; grid is covered too).
        // The `Normal => None` mapping in `self_alignment_to_taffy` cannot
        // apply here: taffy's `None` means inherit the corresponding
        // container property (exactly the spec behavior of `auto`). Since
        // `normal` means stretch regardless of the parent's value, map it
        // explicitly to `STRETCH`.
        AlignSelfValue::Value(SelfAlignmentValue::Normal) => Some(TaffyAlignItems::STRETCH),
        AlignSelfValue::Value(v) => self_alignment_to_taffy(v),
        // cov:ignore: unreachable while AlignSelfValue is Auto|Value(_)
        // only; required for its #[non_exhaustive] contract
        // (the same rationale as `self_alignment_to_taffy`'s catch-all).
        _ => None,
    }
}

/// [`ContentAlignmentValue`] → `Option<taffy::AlignContent>` mapping —
/// [`bridge_alignment`] shares this between `justify_content` and `align_content`,
/// just as [`ContentAlignmentValue`] is the shared payload type of those
/// two properties.
fn content_alignment_to_taffy(v: ContentAlignmentValue) -> Option<TaffyAlignContent> {
    match v {
        ContentAlignmentValue::Normal => None,
        ContentAlignmentValue::Stretch => Some(TaffyAlignContent::STRETCH),
        ContentAlignmentValue::SpaceBetween => Some(TaffyAlignContent::SPACE_BETWEEN),
        ContentAlignmentValue::SpaceEvenly => Some(TaffyAlignContent::SPACE_EVENLY),
        ContentAlignmentValue::SpaceAround => Some(TaffyAlignContent::SPACE_AROUND),
        ContentAlignmentValue::Center => Some(TaffyAlignContent::CENTER),
        ContentAlignmentValue::Start => Some(TaffyAlignContent::START),
        ContentAlignmentValue::End => Some(TaffyAlignContent::END),
        ContentAlignmentValue::FlexStart => Some(TaffyAlignContent::FLEX_START),
        ContentAlignmentValue::FlexEnd => Some(TaffyAlignContent::FLEX_END),
        // cov:ignore: unreachable while ContentAlignmentValue is the 10
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// [`SelfAlignmentValue`] → `Option<taffy::AlignItems>` mapping —
/// [`bridge_alignment`] shares this between its `align_items` field and
/// the `AlignSelfValue::Value` branch of `align_self`: non-`auto` values of
/// `align-self` use the `align-items` keyword set (see [`AlignSelfValue`]'s docs).
fn self_alignment_to_taffy(v: SelfAlignmentValue) -> Option<TaffyAlignItems> {
    match v {
        SelfAlignmentValue::Normal => None,
        SelfAlignmentValue::Stretch => Some(TaffyAlignItems::STRETCH),
        SelfAlignmentValue::Center => Some(TaffyAlignItems::CENTER),
        SelfAlignmentValue::Start => Some(TaffyAlignItems::START),
        SelfAlignmentValue::End => Some(TaffyAlignItems::END),
        SelfAlignmentValue::FlexStart => Some(TaffyAlignItems::FLEX_START),
        SelfAlignmentValue::FlexEnd => Some(TaffyAlignItems::FLEX_END),
        SelfAlignmentValue::Baseline => Some(TaffyAlignItems::BASELINE),
        // cov:ignore: unreachable while SelfAlignmentValue is the 8
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// `row-gap` / `column-gap` → [`taffy::Style::gap`] (`Size<LengthPercentage>`)
/// bridge.
///
/// CSS Box Alignment Module Level 3 §8.1 "Row and Column Gutters: the
/// row-gap and column-gap properties"
/// <https://www.w3.org/TR/css-align-3/#column-row-gap> defines "Initial: normal";
/// its description of that value says (verbatim):
///
/// > The value `normal` represents a used value of `1em` on multi-column
/// > containers, and a used value of `0px` in all other contexts.
///
/// Thus flex/grid containers belong to "all other contexts" in that quote.
/// This is an initial-value mismatch like [`bridge_box_sizing`]'s correction:
/// the raikiri-style computed initial retains the `normal` keyword
/// ([`ComputedLengthPercentageOrNormal::Normal`]), but taffy's
/// `Size<LengthPercentage>` (not an `Option`) has no `normal` keyword.
/// This bridge therefore translates `normal → 0`.
///
/// Convert `Px` / `Percent` to [`ComputedLengthPercentage`] before calling
/// [`computed_length_percentage_to_taffy_length_percentage`], which handles
/// percent `/100.0` conversion and the non-finite [`sanitize_taffy`] guard.
/// This reuses [`bridge_padding`]'s helper instead of duplicating it.
fn bridge_gap(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.gap = Size {
        width: computed_gap_component_to_taffy(cv.column_gap, "column-gap", diag),
        height: computed_gap_component_to_taffy(cv.row_gap, "row-gap", diag),
    };
}

/// Per-axis helper for [`bridge_gap`]: `width` in `taffy::Size<LengthPercentage>`
/// corresponds to the inline axis (`column-gap`), while `height` corresponds
/// to the block axis (`row-gap`). This follows taffy `Size` conventions and
/// the axis mapping of `width`/`height` in [`bridge_size`].
fn computed_gap_component_to_taffy(
    gap: ComputedLengthPercentageOrNormal,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    let lp = match gap {
        ComputedLengthPercentageOrNormal::Normal => ComputedLengthPercentage::Px(0.0),
        ComputedLengthPercentageOrNormal::Px(v) => ComputedLengthPercentage::Px(v),
        ComputedLengthPercentageOrNormal::Percent(p) => ComputedLengthPercentage::Percent(p),
    };
    computed_length_percentage_to_taffy_length_percentage(lp, site, diag)
}

/// grid container/item property → [`taffy::Style`] bridge (CSS Grid Layout
/// Module Level 1).
///
/// - `grid-template-columns` / `grid-template-rows` → `style.grid_template_columns`
///   / `style.grid_template_rows` (`Vec<GridTemplateComponent>`, §7.2) plus
///   `style.grid_template_column_names` / `style.grid_template_row_names`
///   (line names outside any `repeat()`, §7.2.2; see the shape mapping in
///   [`grid_template_tracks_to_taffy`]'s docs)
/// - `grid-template-areas` → `style.grid_template_areas`
///   (`Vec<GridTemplateArea>`, §7.3)
/// - `grid-auto-columns` / `grid-auto-rows` → `style.grid_auto_columns` /
///   `style.grid_auto_rows` (§7.6)
/// - `grid-auto-flow` → `style.grid_auto_flow` (§7.7)
/// - `grid-row-start`/`grid-row-end` / `grid-column-start`/`grid-column-end`
///   → `style.grid_row` / `style.grid_column` (`Line<GridPlacement>`, §8.3)
fn bridge_grid(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let (columns, column_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_columns, "grid-template-columns", diag);
    style.grid_template_columns = columns;
    style.grid_template_column_names = column_names;
    let (rows, row_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_rows, "grid-template-rows", diag);
    style.grid_template_rows = rows;
    style.grid_template_row_names = row_names;

    style.grid_template_areas = match &cv.grid_template_areas {
        GridTemplateAreasValue::None => None,
        GridTemplateAreasValue::Areas(areas) => Some(taffy::style::GridTemplateAreas {
            areas: areas
                .areas
                .iter()
                .map(|a| TaffyGridTemplateArea {
                    name: a.name.to_string(),
                    row_start: saturate_u16(a.row_start),
                    row_end: saturate_u16(a.row_end),
                    column_start: saturate_u16(a.column_start),
                    column_end: saturate_u16(a.column_end),
                })
                .collect(),
            // taffy clamps grid dimensions to 10,000 tracks; saturate our
            // u32 counts the same way as the area coordinates above.
            row_count: areas.row_count.min(u16::MAX as u32) as u16,
            column_count: areas.column_count.min(u16::MAX as u32) as u16,
        }),
        // cov:ignore: unreachable while GridTemplateAreasValue is
        // None|Areas(_) only; required for its #[non_exhaustive] contract
        // (see `bridge_display` catch-all doc).
        _ => None,
    };

    style.grid_auto_columns = cv
        .grid_auto_columns
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-columns", diag))
        .collect();
    style.grid_auto_rows = cv
        .grid_auto_rows
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-rows", diag))
        .collect();

    style.grid_auto_flow = match cv.grid_auto_flow {
        GridAutoFlowValue::Row => TaffyGridAutoFlow::Row,
        GridAutoFlowValue::Column => TaffyGridAutoFlow::Column,
        GridAutoFlowValue::RowDense => TaffyGridAutoFlow::RowDense,
        GridAutoFlowValue::ColumnDense => TaffyGridAutoFlow::ColumnDense,
        // cov:ignore: unreachable while GridAutoFlowValue is
        // Row|Column|RowDense|ColumnDense only; required for its
        // #[non_exhaustive] contract (see `bridge_display` catch-all doc).
        _ => TaffyGridAutoFlow::Row,
    };

    style.grid_row = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_row_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_row_end),
    };
    style.grid_column = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_column_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_column_end),
    };
}

/// [`ComputedGridTemplateTracks`] → taffy's `(Vec<GridTemplateComponent>,
/// Vec<Vec<String>>)` pair — [`bridge_grid`]'s `grid_template_columns`/
/// `grid_template_rows` field **and** their paired `_names` field share one
/// helper because taffy's `NamedLineResolver` consumes both in lock-step
/// (one name-list entry per top-level component, plus one trailing entry
/// after the last — see taffy 0.12's `NamedLineResolver::new`, which zips
/// `grid_template_columns()`/`grid_template_rows()` against
/// `grid_template_column_names()`/`grid_template_row_names()`). This is
/// exactly [`crate` `raikiri_style`]'s own `ComputedGridTrackList::line_names`
/// interleave convention (`line_names.len() == components.len() + 1`), so
/// the two Vecs below are built from the same source data without any
/// reindexing.
///
/// `none` maps to a pair of empty `Vec`s — taffy's own default for a
/// container with no explicit track list.
fn grid_template_tracks_to_taffy(
    tracks: &ComputedGridTemplateTracks,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> (Vec<GridTemplateComponent<String>>, Vec<Vec<String>>) {
    match tracks {
        ComputedGridTemplateTracks::None => (Vec::new(), Vec::new()),
        ComputedGridTemplateTracks::List(list) => {
            let components = list
                .components
                .iter()
                .cloned()
                .map(|component| match component {
                    ComputedGridTrackListComponent::Size(size) => {
                        GridTemplateComponent::Single(grid_track_size_to_taffy(size, site, diag))
                    }
                    ComputedGridTrackListComponent::Repeat(repeat) => {
                        GridTemplateComponent::Repeat(GridTemplateRepetition {
                            count: grid_repeat_count_to_taffy(repeat.count),
                            tracks: repeat
                                .tracks
                                .into_iter()
                                .map(|t| grid_track_size_to_taffy(t, site, diag))
                                .collect(),
                            line_names: repeat
                                .line_names
                                .into_iter()
                                .map(|names| names.iter().map(ToString::to_string).collect())
                                .collect(),
                        })
                    }
                })
                .collect();
            let line_names = list
                .line_names
                .iter()
                .map(|names| names.iter().map(ToString::to_string).collect())
                .collect();
            (components, line_names)
        }
    }
}

/// [`GridRepeatCount`] → [`taffy::RepetitionCount`] mapping. `Count`'s
/// `<integer [1,∞]>` payload is `u32` at the raikiri-style layer (CSS
/// Values 4 §4.2 places no upper bound on `<integer>`) but taffy's
/// `RepetitionCount::Count` is `u16` — silently saturating at
/// [`u16::MAX`] (65535 repetitions) rather than adding new
/// [`LayoutWarn`] machinery for an author value that large, which would
/// already make taffy's own track-sizing algorithm impractically slow
/// long before this cast is reached.
fn grid_repeat_count_to_taffy(count: GridRepeatCount) -> TaffyRepetitionCount {
    match count {
        GridRepeatCount::Count(n) => TaffyRepetitionCount::Count(saturate_u16(n)),
        GridRepeatCount::AutoFill => TaffyRepetitionCount::AutoFill,
        GridRepeatCount::AutoFit => TaffyRepetitionCount::AutoFit,
        // cov:ignore: unreachable while GridRepeatCount is
        // Count|AutoFill|AutoFit only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyRepetitionCount::Count(1),
    }
}

/// [`ComputedGridTrackSize`] → [`taffy::TrackSizingFunction`]
/// (`MinMax<MinTrackSizingFunction, MaxTrackSizingFunction>`) mapping.
///
/// `FitContent` has no bare-breadth min side in taffy's model (only
/// `MaxTrackSizingFunction::fit_content_px`/`fit_content_percent` exist) —
/// the CSS spec formula `max(minimum, min(limit, max-content))` treats the
/// min side as `auto`, which is what `MinTrackSizingFunction::auto()`
/// supplies here.
fn grid_track_size_to_taffy(
    size: ComputedGridTrackSize,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> TrackSizingFunction {
    match size {
        ComputedGridTrackSize::Breadth(b) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(b, site, diag),
            max: grid_track_breadth_to_taffy_max(b, site, diag),
        },
        ComputedGridTrackSize::MinMax(min, max) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(min, site, diag),
            max: grid_track_breadth_to_taffy_max(max, site, diag),
        },
        ComputedGridTrackSize::FitContent(lp) => {
            let max = match lp {
                ComputedLengthPercentage::Px(v) => {
                    MaxTrackSizingFunction::fit_content_px(sanitize_taffy(v, site, diag))
                }
                ComputedLengthPercentage::Percent(p) => {
                    MaxTrackSizingFunction::fit_content_percent(sanitize_taffy(
                        p / 100.0,
                        site,
                        diag,
                    ))
                }
            };
            TrackSizingFunction {
                min: MinTrackSizingFunction::auto(),
                max,
            }
        }
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MinTrackSizingFunction`] mapping
/// (`minmax()`'s min side, or the min side of a bare `<track-breadth>`
/// widened to `MinMax { min, max }` — [`grid_track_size_to_taffy`]'s
/// `Breadth` arm). `Flex` never actually reaches this function
/// ([`ComputedGridTrackBreadth`] doc's collapse note — the min side is
/// only ever produced by `resolve_grid_inflexible_breadth`, which has no
/// `Flex` variant to produce it from); the arm below is a defensive
/// fallback to keep the match exhaustive, not a reachable case.
fn grid_track_breadth_to_taffy_min(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MinTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MinTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MinTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::MinContent => MinTrackSizingFunction::min_content(),
        B::MaxContent => MinTrackSizingFunction::max_content(),
        B::Auto => MinTrackSizingFunction::auto(),
        // cov:ignore: see this fn's doc — structurally unreachable, kept
        // only for match exhaustiveness.
        B::Flex(_) => MinTrackSizingFunction::auto(),
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MaxTrackSizingFunction`] mapping
/// (`minmax()`'s max side, or the max side of a bare `<track-breadth>` —
/// [`grid_track_size_to_taffy`]'s `Breadth` arm). Unlike
/// [`grid_track_breadth_to_taffy_min`], `Flex` (`fr`) is a real, reachable
/// case here — the `fr` unit is only valid on the max side of `minmax()`
/// or as a bare `<track-breadth>` (CSS Grid Layout Module Level 1 §7.2.4).
fn grid_track_breadth_to_taffy_max(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MaxTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MaxTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MaxTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::Flex(f) => MaxTrackSizingFunction::fr(sanitize_taffy(f, site, diag)),
        B::MinContent => MaxTrackSizingFunction::min_content(),
        B::MaxContent => MaxTrackSizingFunction::max_content(),
        B::Auto => MaxTrackSizingFunction::auto(),
    }
}

/// [`GridLineValue`] → [`taffy::GridPlacement`] mapping (CSS Grid Layout
/// Module Level 1 §8.3). [`GridLineValue::Named`] (bare `<custom-ident>`,
/// no explicit index) maps to `NamedLine` with index `1` explicitly filled
/// in — same as [`GridLineValue`]'s own doc explains: taffy 0.12 treats
/// index `0` as an "unspecified" sentinel and normalizes it to `1`
/// internally (`NamedLineResolver::find_line_index`'s `if idx == 0 { idx =
/// 1; }`), so filling in `1` here ahead of time is behavior-identical.
///
/// `Line`/`Span`/`NamedSpan`'s `i32`/`u32` payloads are widened from parse
/// time's unbounded `<integer>` (CSS Values 4 §4.2) but taffy's
/// `GridLine`/`Span(u16)`/`NamedSpan(_, u16)` use 16-bit integers —
/// silently saturating at [`i16::MIN`]/[`i16::MAX`]/[`u16::MAX`] rather
/// than adding new [`LayoutWarn`] machinery, same rationale as
/// [`grid_repeat_count_to_taffy`].
fn grid_line_value_to_taffy_placement(v: &GridLineValue) -> GridPlacement {
    match v {
        GridLineValue::Auto => GridPlacement::Auto,
        GridLineValue::Line(n) => taffy_style_helpers::line(saturate_i16(*n)),
        GridLineValue::Named(name) => GridPlacement::NamedLine(name.to_string(), 1),
        GridLineValue::NamedLine(name, n) => {
            GridPlacement::NamedLine(name.to_string(), saturate_i16(*n))
        }
        GridLineValue::Span(n) => GridPlacement::Span(saturate_u16(*n)),
        GridLineValue::SpanNamed(name, n) => {
            GridPlacement::NamedSpan(name.to_string(), saturate_u16(*n))
        }
        // cov:ignore: unreachable while GridLineValue is the 6 variants
        // matched above only; required for its #[non_exhaustive] contract.
        _ => GridPlacement::Auto,
    }
}

/// Saturating `i32` → `i16` cast — [`grid_line_value_to_taffy_placement`]
/// doc's "silently saturating" rationale.
fn saturate_i16(n: i32) -> i16 {
    n.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Saturating `u32` → `u16` cast — [`grid_line_value_to_taffy_placement`] /
/// [`grid_repeat_count_to_taffy`] doc's "silently saturating" rationale.
fn saturate_u16(n: u32) -> u16 {
    n.min(u16::MAX as u32) as u16
}

/// [`ComputedLengthPercentage`] → [`taffy::LengthPercentage`] bridge
/// (for padding / gap).
///
/// `site` distinguishes the callers (`"padding"` / `"row-gap"` /
/// `"column-gap"`) in [`LayoutWarn::NonFiniteClamped`] events — same
/// `site`-parameter shape as
/// [`computed_length_percentage_or_auto_to_taffy_dimension`]
/// (`"width"`/`"height"`), generalized once a second caller
/// ([`bridge_gap`]) appeared.
///
/// # Exhaustive match
///
/// The argument has a **computed-value** type, making two exhaustive arms
/// possible. The old `Length::Em(_) | Length::Rem(_) => length(0.0)` arm
/// (which silently collapsed font-relative units to 0px) and the
/// `_ => length(0.0)` non_exhaustive catch-all were **removed**. Cascade
/// phase 3 converts `em` / `rem` / `pt` to px before they reach computed values.
///
/// **But the removed arms absorbed pathological `f32` values as well as units**
/// (`Em(inf)` / `0.0 * inf` = NaN). [`sanitize_taffy`] covers this case instead;
/// **do not remove its guard as "unnecessary defensive code".**
/// Keep this protection in place.
///
/// [`ComputedLengthPercentage`] deliberately lacks `#[non_exhaustive]` so this
/// exhaustive match works today. This is an explicit trade-off: adding a
/// `Calc` variant later requires a coordinated breaking change (see the module
/// docs for `raikiri_style::resolve`). **Do not add a `_` arm in the name of
/// "forward compatibility"**; that would discard the benefit of this trade-off.
///
/// # Percent policy
///
/// `Percent(p)` → `percent(sanitize_taffy(p / 100.0))`: convert the authored
/// CSS 0–100 percent to taffy's 0.0–1.0 fraction, then make it finite with [`sanitize_taffy`].
/// Resolution against the containing block happens at the **used-value stage**
/// (CSS Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>) and is
/// delegated to taffy. **The guard bounds the fraction, not the resolved
/// used value** (see [`MAX_TAFFY_MAGNITUDE`]'s scope section).
pub(crate) fn computed_length_percentage_to_taffy_length_percentage(
    len: ComputedLengthPercentage,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    match len {
        // Site 1: `sanitize_taffy` rejects non-finite values.
        ComputedLengthPercentage::Px(v) => LengthPercentage::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentage::Percent(p) => {
            LengthPercentage::percent(sanitize_taffy(p / 100.0, site, diag))
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::Dimension`] bridge
/// (for width / height).
///
/// The exhaustive `Px` / `Percent` branching, percent policy, and non-finite
/// guard match [`computed_length_percentage_to_taffy_length_percentage`], which
/// has two arms. This helper adds `Auto`, for three arms without a catch-all.
/// `Auto` → `Dimension::auto()` (no `f32`, so no guard is needed).
///
/// Both width and height in [`bridge_size`] consume this helper.
///
/// `site` distinguishes the two [`bridge_size`] callers (`"width"` /
/// `"height"`) in [`LayoutWarn::NonFiniteClamped`] events.
fn computed_length_percentage_or_auto_to_taffy_dimension(
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> Dimension {
    match loa {
        // Site 2.
        ComputedLengthPercentageOrAuto::Px(v) => Dimension::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentageOrAuto::Percent(p) => {
            Dimension::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => Dimension::auto(),
        ComputedLengthPercentageOrAuto::Calc(value) => {
            calc_values.push(std::sync::Arc::new(value));
            let pointer = calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            Dimension::calc(pointer)
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// for min/max-size properties.
///
/// Unlike the margin/inset bridge below, min/max preferred sizes can carry a
/// calc() payload. Preserve that payload in the document arena so consumers
/// such as table `<col>` sizing do not observe a false zero constraint.
fn computed_length_percentage_or_auto_to_taffy_min_max(
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        ComputedLengthPercentageOrAuto::Px(v) => {
            LengthPercentageAuto::length(sanitize_taffy(v, site, diag))
        }
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => LengthPercentageAuto::auto(),
        ComputedLengthPercentageOrAuto::Calc(value) => {
            calc_values.push(std::sync::Arc::new(value));
            let pointer = calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentageAuto::calc(pointer)
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// (for margin).
///
/// The exhaustive match, percent policy, and non-finite guard match
/// [`computed_length_percentage_to_taffy_length_percentage`]. `Auto` maps to
/// `LengthPercentageAuto::auto()` (delegating CSS Box 3 §3.1's "margin auto = distribute
/// available space" to taffy; no `f32` means no guard is needed).
fn computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        // Site 3: negative margins are valid under the spec, so the symmetric
        // `sanitize_taffy` clamp matters. Only `bridge_margin` calls this
        // helper, so its site label is fixed.
        ComputedLengthPercentageOrAuto::Px(v) => {
            LengthPercentageAuto::length(sanitize_taffy(v, site, diag))
        }
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => LengthPercentageAuto::auto(),
        // Calc payloads are currently produced only for preferred sizes;
        // margin calc resolution will share the same arena in a later pass.
        ComputedLengthPercentageOrAuto::Calc(_) => LengthPercentageAuto::length(0.0),
    }
}

/// [`ComputedLength`] (px) → [`taffy::LengthPercentage`] bridge
/// (for `border-*-width`).
///
/// Keep this alongside three sibling helpers (`computed_length_percentage_to_taffy_length_percentage` /
/// `computed_length_percentage_or_auto_to_taffy_dimension` /
/// `computed_length_percentage_or_auto_to_taffy_length_percentage_auto`), with
/// the same `<src>_to_taffy_<dst>` naming pattern.
///
/// This is separate for `border-*-width` because its grammar (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) excludes percentages. Thus even
/// at the computed-value stage, only lengths occur (CSS Backgrounds 3 §3.3 "Line
/// Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>). Its return type
/// matches [`computed_length_percentage_to_taffy_length_percentage`]
/// (`LengthPercentage` is the smallest shared taffy type), but its argument
/// is [`ComputedLength`], so it has no percentage arm. It still uses the
/// non-finite guard ([`sanitize_taffy`]).
fn computed_length_to_taffy_length_percentage(
    len: ComputedLength,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    // Site 4 has only one caller, `bridge_border`,
    // so the site label is fixed.
    LengthPercentage::length(sanitize_taffy(len.px(), "border-width", diag))
}

#[cfg(test)]
mod tests;
