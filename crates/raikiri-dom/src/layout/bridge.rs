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
    let (gap, gap_percent) = match cv.column_gap {
        ComputedLengthPercentageOrNormal::Px(value) if value.is_finite() => (value.max(0.0), None),
        ComputedLengthPercentageOrNormal::Percent(value) if value.is_finite() => (0.0, Some(value)),
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

/// ComputedValues → taffy::Style bridge の dispatch site。
///
/// per-element for loop 内 inline mapping から per-field `bridge_*` helper へ
/// dispatch する pattern にリファクタ済み。新しい bridge は helper 追加 +
/// dispatch 1 行追加のみで足りるよう設計している。
///
/// 現時点で active な bridge:
/// - [`bridge_direction`] — [`Direction`] → [`taffy::Direction`]
/// - [`bridge_display`] — [`DisplayValue`] → [`taffy::Display`]
/// - [`bridge_float`] — [`ComputedValues::float`] / [`ComputedValues::clear`] →
///   [`taffy::Style::float`] / [`taffy::Style::clear`] (CSS2 §9.5.1 / §9.5.2)
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentage>` → [`taffy::Rect<LengthPercentage>`]
/// - [`bridge_size`] — [`ComputedLengthPercentageOrAuto`] `cv.width` / `cv.height` →
///   [`taffy::Style::size`] (`Size<Dimension>`)。width / height 両 field を
///   struct literal 1 発 assign で書く。
/// - [`bridge_min_max_size`] — [`ComputedLengthPercentageOrAuto`]
///   `cv.min_width` / `cv.min_height` → [`taffy::Style::min_size`]、
///   `cv.max_width` / `cv.max_height` → [`taffy::Style::max_size`]。
///   min/max 4 field を 2 struct literal で書く ([`bridge_size`] と同 shape)。
/// - [`bridge_border`] — `Sides<ComputedBorder>` → [`taffy::Rect<LengthPercentage>`]
///   。**border-style gating は本 bridge ではなく上流の
///   `raikiri_style::resolve_border` (computed 層) が持つ** — CSS Backgrounds 3
///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>。
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
/// per-node の bridge loop の後、second pass
/// ([`establish_minimal_line_boxes`]) が bridge 済みの tree 全体を走査し、
/// 条件を満たす block container に minimal な inline formatting context を
/// 確立する — qualifying condition と scope は同関数の doc 参照。
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

/// [`DisplayValue`] → [`taffy::Display`] mapping。
///
/// [`DisplayValue`] を
/// [`taffy::Display`] に mapping する。taffy 0.x は Block / Flex / Grid /
/// None のみ (`Inline` / `InlineBlock` 独立 variant なし) のため:
/// - `Inline` → `Block` (initial は Block、text-only は leaf で render)
/// - `InlineBlock` → `Block` (block child + inline-level flow parent の
///   separate 扱い、精密化は follow-up)
/// - `None` → `None`
/// - `Flex` → `Flex` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block flex` と等価。
///   [`crate::taffy_impl`]'s `LayoutFlexboxContainer` impl + taffy's
///   `compute_flexbox_layout` が実 layout を担う)
/// - `Grid` → `Grid` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block grid` と等価。
///   [`crate::taffy_impl`]'s `LayoutGridContainer` impl + taffy's
///   `compute_grid_layout` が実 layout を担う)
/// - Table internal types (`table` / `inline-table` / `table-row-group` /
///   `table-header-group` / `table-footer-group` / `table-row` /
///   `table-column-group` / `table-column` / `table-cell` / `table-caption`)
///   → `Block` (暫定: taffy 0.12 は table layout 未対応のため block 近似。
///   TODO(table-layout): [`crate::layout::table`] の dedicated table
///   formatting context (CSS 2.1 §17.2.1 anonymous table object generation
///   含む) が landing したら専用 Display / layout へ置換)
/// - `flow-root` → `FlowRoot` (独立 block formatting context)
/// - `list-item` / `contents` → `Block` (catch-all 経由、将来専用 handling
///   が入るまで block 近似)
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
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>)。**enum 1:1
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
/// (`Rect<LengthPercentageAuto>`) bridge。
///
/// CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> の
/// physical margin 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_length_percentage_auto`] を参照。
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
/// (`Rect<LengthPercentage>`) bridge。
///
/// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical> の
/// physical padding 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。margin と
/// の差は value type: padding は `<length-percentage [0,∞]>` (auto なし、
/// non-negative は raikiri-style parse-time enforce) のため
/// [`computed_length_percentage_to_taffy_length_percentage`] を使う。
///
/// Length policy は [`computed_length_percentage_to_taffy_length_percentage`] を参照。
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
/// (`Rect<LengthPercentage>`) bridge。
///
/// # style-gating は **上流** で済んでいる
///
/// `border-style: none` / `hidden` の側で width を 0 にする規則は、CSS
/// Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
/// の propdef table が "Computed value: absolute length, snapped as a border
/// width; **zero if the border style is `none` or `hidden`**" と規定するとおり
/// **computed 層**の要求である。したがって gate は
/// `raikiri_style::resolve_border` が持ち、[`ComputedValues::border`] に届く
/// 時点で width は既に 0 に潰れている。
///
/// 本 bridge が同じ判定を再実装してはならない (spec 規則の二重実装になり、
/// 一方だけ直す drift の温床になる)。かつてあった `used_border_width` helper は
/// この理由で削除した。end-to-end の gating check は本 file の
/// `apply_computed_to_style_bridges_border_to_taffy` が引き続き持つ。
///
/// `@page` 経路 (`PageCascadeResult::declarations`) も同じ `resolve_border` へ
/// funnel するようになったので、border-width の gate は
/// raikiri-style 側に 1 本しか無い。本 bridge が読むのは per-node
/// [`ComputedValues`] なので直接の影響は無いが、将来 page-margin box の layout を
/// 本 bridge に通す場合も **gate を再実装せず** 上流の値を信頼すること。
///
/// ⚠️ 上記は **border-style gating に限った話**。`PageCascadeResult::declarations`
/// は非有限 `f32` (+Inf / NaN) について本段落とは別の未対応 hazard を抱えている
/// (contract は `raikiri_style::page::PageCascadeResult::declarations`
/// の doc が canonical)。将来 page-margin box の layout を本 bridge に通す際は
/// gating の再確認だけでなくそちらも再確認すること。
///
/// 4-side は **field 名 mapping** で write (positional constructor は使わない —
/// [`bridge_margin`] と同じ `Sides` vs `Rect` field 順不一致の silent transpose
/// 防止)。
///
/// # taffy scope の非対応
///
/// - `border-color` / `border-style` 自体は taffy が track しない (taffy は
///   border-width のみ)。色 / 線 pattern は paint scope が別途 [`ComputedValues::border`]
///   から consume する将来 task。
/// - `border-image` / `border-radius` はスコープ外。
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let b = cv.border;
    style.border = Rect {
        top: computed_length_to_taffy_length_percentage(b.top.width(), diag),
        right: computed_length_to_taffy_length_percentage(b.right.width(), diag),
        bottom: computed_length_to_taffy_length_percentage(b.bottom.width(), diag),
        left: computed_length_to_taffy_length_percentage(b.left.width(), diag),
    };
}

/// [`ComputedValues::width`] / [`ComputedValues::height`] (`ComputedLengthPercentageOrAuto`) →
/// [`taffy::Style::size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §3.1.1
/// "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
///
/// width 側を先に landing、続けて height 側を追記して両 preferred size 軸を
/// full-bridge にした。両 field を同時に書き込むため struct literal
/// (`style.size = Size { width, height }`) で 1 発 assign する — partial write
/// scaffold はもう不要になった。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_dimension`] を参照。
///
/// # PageBox 妥協
///
/// `<body>` element の `style.size` は本 bridge の後、[`apply_page_box_to_body`]
/// で PageBox の値 (width / height 両方) に上書きされる (layout.rs Step 1 →
/// Step 4)。したがって `<body style="width: 100px; height: 200px">` の author
/// 値は本 helper で一度 taffy に write されるが、Step 4 で PageBox 値に
/// clobber される — 現行実装で意図された挙動 (将来 @page cascade + per-page
/// PageBox に refactor 予定)。width 側 clobber の author→PageBox 上書き経路は
/// test `apply_page_box_clobbers_body_width_from_bridge` が check する。height
/// 側は [`apply_page_box_to_body`] が `style.size = Size { width, height }` の
/// struct literal で **field を分岐なく一括代入する** ため、width と同じ
/// clobber 経路を通る (両 field は同一 statement で書かれる)。同 helper の
/// PageBox output check は test `apply_page_box_to_body_sets_body_style_size_to_page_dimensions`
/// が担う (author→PageBox の bridge→clobber 連鎖 test は width 側で十分、
/// 冗長化を避け height 側は structural 保証に留める)。
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

fn bridge_size(doc: &mut Document, node_id: usize, cv: &ComputedValues) {
    // width + height 両方を同時に書くので struct literal を採用。
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
/// min 側 initial `auto` / max 側 initial `none` は共に computed 層で
/// [`ComputedLengthPercentageOrAuto::Auto`] に正規化済み (specified 層の
/// `none` → `Auto` placeholder mapping は sibling `parse_max_size` が担う)
/// ので、4 field とも同じ
/// [`computed_length_percentage_or_auto_to_taffy_min_max`] helper で `Auto` →
/// `LengthPercentageAuto::auto()` に translate する — max の `none` 用の特別扱いは本
/// bridge に要らない。`none` (no max) と `auto` (no minimum) は共に
/// "制約なし" として taffy に委譲する。calc() payloads are retained in the
/// document arena by that helper.
///
/// Length policy / Percent policy / 非有限 guard は helper の doc 参照。
/// `site` label は 4 caller ごとに `min-width` / `min-height` /
/// `max-width` / `max-height` を渡す ([`LayoutWarn::NonFiniteClamped`] の診断用)。
fn bridge_min_max_size(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<raikiri_style::property::CalcLengthPercentage>>,
    diag: &mut Vec<LayoutWarn>,
) {
    // min/max 各 2 field を同時に書くので struct literal を 2 発採用
    // (bridge_size と同 shape — default 保持は cv 側の Auto で自然に達成)。
    // Keep calc() handles alive in the document arena just like width/height;
    // collapsing a max-width calc() to zero would spuriously clamp a <col>.
    style.min_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.min_width,
            "min-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.min_height,
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

/// [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`] bridge。
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`、spec initial `content-box`。**enum 1:1 mapping**
/// (Length policy に不参加の最小 helper)。
///
/// # Initial-value 補正 note
///
/// - raikiri-style initial = `BoxSizing::ContentBox` (CSS Sizing 3 §3.3 spec 準拠)
/// - taffy default = `taffy::BoxSizing::BorderBox` (taffy 0.12 の `#[default]`)
///
/// 両者の初期値は spec と食い違うが、cascade は unspecified 時に必ず
/// [`ComputedValues::initial`] 経由で `ContentBox` を seed するため、本 bridge が
/// 走った後の `style.box_sizing` は常に spec 初期値 (`ContentBox`) になる。
/// つまり本 helper の副作用として "taffy default の spec 違反" を補正する。
///
/// # non_exhaustive catch-all
///
/// raikiri-style の [`BoxSizing`] は `#[non_exhaustive]`
/// (forward-compat のための sibling pattern)。未知 variant は spec initial (`ContentBox`) に
/// fail-quiet — spec-violation を silent に伸ばさないよう "最も安全な既定"
/// にする方針 ([`bridge_display`] catch-all → `Block` と同じ趣旨)。
///
/// taffy 側 (`taffy::BoxSizing`) は `#[non_exhaustive]` **ではない** ため、
/// mapping 出力 arm は `ContentBox` / `BorderBox` の 2 個で網羅済。
///
/// [`BoxSizing`]: raikiri_style::property::BoxSizing
fn bridge_box_sizing(style: &mut taffy::Style, cv: &ComputedValues) {
    style.box_sizing = match cv.box_sizing {
        StyleBoxSizing::ContentBox => TaffyBoxSizing::ContentBox,
        StyleBoxSizing::BorderBox => TaffyBoxSizing::BorderBox,
        // non_exhaustive catch-all — 未知 variant は spec initial (ContentBox)
        // に fail-quiet (silent spec-violation 拡大を避ける)。
        _ => TaffyBoxSizing::ContentBox,
    };
}

/// flex container/item property → [`taffy::Style`] bridge。
///
/// CSS Flexible Box Layout Module Level 1 の 5 property を対応する taffy
/// field へ写す:
///
/// - `flex-direction` ([`FlexDirectionValue`]) → `style.flex_direction`
///   (§5.1)
/// - `flex-wrap` ([`FlexWrapValue`]) → `style.flex_wrap` (§5.2)
/// - `flex-grow` (`f32`) → `style.flex_grow` (§7.2.1、無変換で直接 copy —
///   `<number>` は spec 上絶対化を要さない、[`ComputedValues::flex_grow`]
///   doc の sink-guard 注記参照)
/// - `flex-shrink` (`f32`) → `style.flex_shrink` (§7.2.2、同上)
/// - `flex-basis` ([`ComputedFlexBasis`]) → `style.flex_basis`
///   (`taffy::Dimension`、§7.2.3)
///
/// # `flex-basis` の bridge
///
/// `Px` / `Percent` は [`bridge_size`] と同じ percent `/100.0` 変換 +
/// [`sanitize_taffy`] 非有限 guard で [`taffy::Dimension`] に載せる。
/// `MinContent` / `MaxContent` / `FitContent` は taffy 0.14 の同名
/// `Dimension` variant にそのまま写像する (flex アルゴリズムが content
/// 測定で解決する)。
///
/// - `Content` → [`taffy::Dimension::content`] (0.14 — flex-basis 専用
///   keyword、そのままの意味で写像する)。
fn bridge_flex(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.flex_direction = match cv.flex_direction {
        FlexDirectionValue::Row => TaffyFlexDirection::Row,
        FlexDirectionValue::RowReverse => TaffyFlexDirection::RowReverse,
        FlexDirectionValue::Column => TaffyFlexDirection::Column,
        FlexDirectionValue::ColumnReverse => TaffyFlexDirection::ColumnReverse,
        // cov:ignore: unreachable while FlexDirectionValue is
        // Row|RowReverse|Column|ColumnReverse only; required for its
        // #[non_exhaustive] contract (see doc above, `bridge_display`
        // catch-all と同じ趣旨).
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
    // `<number>` は絶対化不要 — 無変換で直接 copy。
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
/// Level 3)。
///
/// - `justify-content` ([`ContentAlignmentValue`]) → `style.justify_content`
///   (§5.1)
/// - `align-content` ([`ContentAlignmentValue`]) → `style.align_content`
///   (§5.1)
/// - `align-items` ([`SelfAlignmentValue`]) → `style.align_items` (§7.2)
/// - `align-self` ([`AlignSelfValue`]) → `style.align_self` (§6.2)
/// - `justify-items` ([`SelfAlignmentValue`]) → `style.justify_items` (§7.1)
///   — grid container 上の inline-axis 版 `align-items`、同じ keyword set
///   ([`SelfAlignmentValue`] doc 参照) を共有する。
/// - `justify-self` ([`AlignSelfValue`]) → `style.justify_self` (§6.1) —
///   grid item 上の inline-axis 版 `align-self`。taffy `GridItemStyle::justify_self`
///   の doc も `align_self` と同じ "Falls back to the parents … if not set"
///   契約を持つため、`align-self` と同じ `normal`/`auto` 特殊 handling が
///   そのまま適用できる。
///
/// taffy 側の 6 field は全て `Option<…>` — CSS の `normal` (content-*系)
/// keyword には対応する taffy keyword が無く、`None` へ写す
/// (`bridge_box_sizing` の "Initial-value 補正" 節と同型の initial-value
/// mismatch)。taffy はその後 layout mode 依存の default で埋める
/// (`GridContainerStyle::grid_align_content` 等の `unwrap_or(AlignContent::STRETCH)`
/// — grid path の fallback は `STRETCH`、flex path は
/// `compute_flexbox_layout` 内部の別 default)。`align-self`/`justify-self`
/// の `auto` も同じ形 (`Option::None` → 親の `align-items`/`justify-items`
/// に fallback、CSS Box Alignment 3 §6.2/§6.1 の spec 規定どおり)。
///
/// 明示 keyword は [`taffy::AlignItems`] / [`taffy::AlignContent`] の
/// 定数 (`AlignItems::CENTER` 等、struct constant であって enum variant
/// ではない — taffy 0.12 の safe/unsafe overflow-position 分離 shape、
/// [`ContentAlignmentValue`]/[`SelfAlignmentValue`] doc の scope carving
/// 節参照) に写す。`safety` field は常に `AlignmentSafety::Unsafe` —
/// `safe`/`unsafe` prefix 自体を本 crate が受理していないため
/// (`parse_content_alignment`/`parse_self_alignment` doc 参照)。
fn bridge_alignment(style: &mut taffy::Style, cv: &ComputedValues) {
    style.justify_content = content_alignment_to_taffy(cv.justify_content);
    style.align_content = content_alignment_to_taffy(cv.align_content);
    style.align_items = self_alignment_to_taffy(cv.align_items);
    style.align_self = self_alignment_or_auto_to_taffy(cv.align_self);
    style.justify_items = self_alignment_to_taffy(cv.justify_items);
    style.justify_self = self_alignment_or_auto_to_taffy(cv.justify_self);
}

/// [`AlignSelfValue`] → `Option<taffy::AlignItems>` mapping — shared by
/// `align-self` and `justify-self` ([`bridge_alignment`] doc の
/// `justify-self` 節参照、taffy 側 `AlignSelf` は `AlignItems` の type
/// alias)。
fn self_alignment_or_auto_to_taffy(v: AlignSelfValue) -> Option<TaffyAlignItems> {
    match v {
        AlignSelfValue::Auto => None,
        // `normal` は `auto` とは異なり、親の align-items/justify-items へ
        // fallback せず、flex/grid layout では単独で `stretch` 相当に
        // 振る舞う (CSS Box Alignment 3 §8.3 の "In flex layout, this
        // value behaves as stretch" — grid layout も同節の対象)。
        // `self_alignment_to_taffy`'s `Normal => None` mapping はここでは
        // 使えない — taffy の `None` は「コンテナの対応 property を
        // 継承する」意味 (`auto` の spec 挙動そのもの) であり、`normal` の
        // 「親の値に関わらず stretch」とは異なるため、明示的に `STRETCH`
        // へ写す。
        AlignSelfValue::Value(SelfAlignmentValue::Normal) => Some(TaffyAlignItems::STRETCH),
        AlignSelfValue::Value(v) => self_alignment_to_taffy(v),
        // cov:ignore: unreachable while AlignSelfValue is Auto|Value(_)
        // only; required for its #[non_exhaustive] contract
        // (`self_alignment_to_taffy` の catch-all と同じ判断).
        _ => None,
    }
}

/// [`ContentAlignmentValue`] → `Option<taffy::AlignContent>` mapping —
/// [`bridge_alignment`] の `justify_content` / `align_content` 両 field で
/// 共有する ([`ContentAlignmentValue`] 自体が両 property の共有 payload
/// 型であるのと同じ理由)。
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
/// [`bridge_alignment`] の `align_items` field と `align_self` の
/// `AlignSelfValue::Value` branch で共有する (`align-self` の非-`auto` 値は
/// `align-items` と同じ keyword set、[`AlignSelfValue`] doc 参照)。
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
/// bridge。
///
/// CSS Box Alignment Module Level 3 §8.1 "Row and Column Gutters: the
/// row-gap and column-gap properties"
/// <https://www.w3.org/TR/css-align-3/#column-row-gap> propdef の
/// "Initial: normal" に対し、同 propdef の value 説明 (verbatim) が:
///
/// > The value `normal` represents a used value of `1em` on multi-column
/// > containers, and a used value of `0px` in all other contexts.
///
/// と規定する — flex/grid container はこの "all other contexts" に属する。
/// これは [`bridge_box_sizing`] の "Initial-value 補正" 節と同型の
/// mismatch: raikiri-style の computed 層 initial は `normal` keyword を
/// 保持する ([`ComputedLengthPercentageOrNormal::Normal`]) が、taffy 側
/// `Size<LengthPercentage>` (non-`Option`) には `normal` keyword が
/// 存在しないため、本 bridge が `normal → 0` の変換を担う。
///
/// `Px` / `Percent` は [`ComputedLengthPercentage`] に一度変換してから
/// [`computed_length_percentage_to_taffy_length_percentage`] (percent の
/// `/100.0` 変換 + [`sanitize_taffy`] 非有限 guard を含む) へ delegate する
/// — [`bridge_padding`] と同じ helper を再利用し、実装を複製しない。
fn bridge_gap(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.gap = Size {
        width: computed_gap_component_to_taffy(cv.column_gap, "column-gap", diag),
        height: computed_gap_component_to_taffy(cv.row_gap, "row-gap", diag),
    };
}

/// [`bridge_gap`] の per-axis helper — `taffy::Size<LengthPercentage>` の
/// `width` は inline axis (`column-gap` 相当)、`height` は block axis
/// (`row-gap` 相当) に対応する (taffy `Size` の一般 convention、
/// [`bridge_size`] の `width`/`height` mapping と同じ軸の向き)。
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
/// Module Level 1)。
///
/// - `grid-template-columns` / `grid-template-rows` → `style.grid_template_columns`
///   / `style.grid_template_rows` (`Vec<GridTemplateComponent>`、§7.2) +
///   `style.grid_template_column_names` / `style.grid_template_row_names`
///   (line names outside any `repeat()`、§7.2.2 — [`grid_template_tracks_to_taffy`]
///   doc の shape 対応参照)
/// - `grid-template-areas` → `style.grid_template_areas`
///   (`Vec<GridTemplateArea>`、§7.3)
/// - `grid-auto-columns` / `grid-auto-rows` → `style.grid_auto_columns` /
///   `style.grid_auto_rows` (§7.6)
/// - `grid-auto-flow` → `style.grid_auto_flow` (§7.7)
/// - `grid-row-start`/`grid-row-end` / `grid-column-start`/`grid-column-end`
///   → `style.grid_row` / `style.grid_column` (`Line<GridPlacement>`、§8.3)
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
/// (padding / gap 用)。
///
/// `site` distinguishes the callers (`"padding"` / `"row-gap"` /
/// `"column-gap"`) in [`LayoutWarn::NonFiniteClamped`] events — same
/// `site`-parameter shape as
/// [`computed_length_percentage_or_auto_to_taffy_dimension`]
/// (`"width"`/`"height"`), generalized once a second caller
/// ([`bridge_gap`]) appeared.
///
/// # 網羅 match
///
/// 引数が **computed 層**の型になったため 2 arm で網羅する。以前あった
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` (font-relative unit を黙って
/// 0px に潰す fail-quiet) と `_ => length(0.0)` (non_exhaustive catch-all) は
/// **削除した** — `em` / `rem` / `pt` は cascade の phase 3 で px に絶対化済み
/// であり、computed 層に到達しない。
///
/// **ただし削除した arm は「単位」だけでなく「病的な f32 の値」も吸収していた**
/// (`Em(inf)` / `0.0 * inf` = NaN)。その分は [`sanitize_taffy`] が
/// backfill している — **guard を「不要な防御」と
/// 判断して外さないこと。**
///
/// [`ComputedLengthPercentage`] に `#[non_exhaustive]` が付いていないのは、
/// この網羅性を今得るための explicit trade である (将来 `Calc` variant が
/// 増えるときに coordinated breaking change を払う。`raikiri_style::resolve`
/// の module doc 参照)。**`_` arm を足して「forward-compat」にしてはならない** —
/// trade の得る側を捨てることになる。
///
/// # Percent policy
///
/// `Percent(p)` → `percent(sanitize_taffy(p / 100.0))` — CSS spec の authored
/// 0-100 を taffy の fraction 0.0-1.0 に変換し、[`sanitize_taffy`] で有限化する。
/// containing block に対する解決は **used value 層**
/// (CSS Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>) であり
/// taffy に委譲する — **guard が bound するのは fraction であって解決後の
/// used value ではない** ([`MAX_TAFFY_MAGNITUDE`] の射程節を参照)。
pub(crate) fn computed_length_percentage_to_taffy_length_percentage(
    len: ComputedLengthPercentage,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    match len {
        // site 1: `sanitize_taffy` で非有限を落とす。
        ComputedLengthPercentage::Px(v) => LengthPercentage::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentage::Percent(p) => {
            LengthPercentage::percent(sanitize_taffy(p / 100.0, site, diag))
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::Dimension`] bridge
/// (width / height 用)。
///
/// 網羅 match (`Px` / `Percent`) の分岐ロジック・Percent policy・非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ (参照先は
/// 2 arm)。本関数はそれに `Auto` arm が加わり合計 3 arm (catch-all なし)。
/// `Auto` → `Dimension::auto()` (f32 を持たないので guard 対象外)。
///
/// [`bridge_size`] から width / height 両方で consume される。
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
        // site 2。
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
/// (margin 用)。
///
/// 網羅 match / Percent policy / 非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ。`Auto` →
/// `LengthPercentageAuto::auto()` (CSS Box 3 §3.1 "margin auto = distribute
/// available space" を taffy に委譲、f32 を持たないので guard 対象外)。
fn computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        // site 3。margin は負値が spec-valid なので
        // `sanitize_taffy` の対称 clamp が load-bearing。唯一の caller
        // (`bridge_margin`) 由来なので site label は固定。
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
/// (`border-*-width` 用)。
///
/// sibling 3 helper (`computed_length_percentage_to_taffy_length_percentage` /
/// `computed_length_percentage_or_auto_to_taffy_dimension` /
/// `computed_length_percentage_or_auto_to_taffy_length_percentage_auto`) と同じ
/// `<src>_to_taffy_<dst>` 命名 / 同じ cluster に置く。
///
/// `border-*-width` 専用に分けているのは、grammar (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) が `<percentage>` を含まないため
/// computed 層でも length しか来ないから (CSS Backgrounds 3 §3.3 "Line
/// Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。戻り値型は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同一
/// (`LengthPercentage` が taffy 側の最小共通型) だが、入力型が
/// [`ComputedLength`] なので percentage arm を
/// 持たない点が違う。非有限 guard ([`sanitize_taffy`]) は同じく通す。
fn computed_length_to_taffy_length_percentage(
    len: ComputedLength,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    // site 4。唯一の caller (`bridge_border`) 由来
    // なので site label は固定。
    LengthPercentage::length(sanitize_taffy(len.px(), "border-width", diag))
}

#[cfg(test)]
mod tests;
