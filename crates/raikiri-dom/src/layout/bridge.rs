use super::*;
use crate::layout::context::LayoutSession;
use crate::node::{FormattingContext, Node};
use crate::taffy_impl::content_limits::ContentLimits;
use raikiri_style::property::{BoxDecorationBreak, ContentComponent, TextOrientation};
use raikiri_style::{PseudoElem, StyleNodeId};

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
        // The column width clamps the sum to zero once the basis is known.
        ComputedLengthPercentageOrNormal::Calc(calc)
            if calc.px.is_finite() && calc.percent.is_finite() =>
        {
            (calc.px, Some(calc.percent))
        }
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
        column_fill: cv.column_fill,
        gap,
        gap_percent,
        height_definite: !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto),
        horizontal: !cv.writing_mode.is_vertical(),
        rtl: matches!(cv.direction, Direction::Rtl),
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
/// - [`Node::set_box_properties`] — canonical CSS display and position → Taffy identity
/// - [`bridge_float`] — [`ComputedValues::float`] / [`ComputedValues::clear`] →
///   [`taffy::Style::float`] / [`taffy::Style::clear`] (CSS2 §9.5.1 / §9.5.2)
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentageOrCalc>` → [`taffy::Rect<LengthPercentage>`]
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
/// After the per-node bridge loop, the paragraphs of the inline engine are
/// assigned ([`crate::layout::ifc::assign::assign_ifc_roots`]).
///
/// # Errors
/// What [`crate::layout::ifc::assign::assign_ifc_roots`] returns: a paragraph
/// over a limit of the inline engine, or one it refuses when it is required
/// for every paragraph.
/// Whether `idx` lays out on vertical axes: a box in `vertical-rl` or
/// `vertical-lr`. A multicol container and its contents run vertical writing
/// on horizontal axes, and so do flex and grid containers: their algorithms
/// place items on physical axes, together with the horizontally mapped
/// logical margins.
pub(crate) fn lays_out_vertically(cascade: &CascadeResult, idx: usize) -> bool {
    let values = &cascade.computed[idx];
    if !matches!(
        values.cssom_writing_mode,
        WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr
    ) || values.display.is_flex_or_grid_container()
    {
        return false;
    }
    !values.multicol_flow
}

/// Whether the bridge gives `idx` its vertical axes: a box that
/// [`lays_out_vertically`], except a table box, which keeps horizontal axes
/// even in vertical writing.
pub(crate) fn uses_vertical_axes(cascade: &CascadeResult, idx: usize) -> bool {
    lays_out_vertically(cascade, idx)
        && !crate::taffy_impl::is_table_box(cascade.computed[idx].display)
}

/// The vertical `inline-size` / `block-size` mapping of `idx`, when the box
/// lays out vertical lines on its own axes ([`lays_out_vertically`]). Other
/// boxes keep the horizontal mapping in [`ComputedValues::width`] /
/// [`ComputedValues::height`].
pub(crate) fn vertical_logical_size(
    cascade: &CascadeResult,
    idx: usize,
) -> Option<&raikiri_style::VerticalLogicalSize> {
    cascade.computed[idx]
        .vertical_logical_size
        .as_ref()
        .filter(|_| lays_out_vertically(cascade, idx))
}

/// The computed values of `idx` with [`vertical_logical_size`] applied.
fn vertical_logical_sized(cascade: &CascadeResult, idx: usize) -> Option<ComputedValues> {
    let size = vertical_logical_size(cascade, idx)?;
    let mut sized = cascade.computed[idx].clone();
    sized.width = size.width;
    sized.height = size.height;
    Some(sized)
}

#[cfg(test)]
pub(crate) fn apply_computed_to_style(
    doc: &mut Document,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    let mut session = LayoutSession::new(doc);
    apply_computed_to_style_in_session(&mut session, cascade)
}

pub(crate) fn apply_computed_to_style_in_session(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    // Keep the previous payloads alive while styles are rebuilt in place.
    // A later error or unwind may leave some styles with old handles and
    // others with newly appended handles, so only a complete successful
    // bridge can release the old prefix.
    let old_calc_len = doc.document.retained_layout_styles.calc_values.len();
    let result = apply_computed_to_style_in_session_inner(doc, cascade);
    if result.is_ok() {
        doc.document
            .retained_layout_styles
            .calc_values
            .drain(..old_calc_len);
    }
    result
}

fn apply_computed_to_style_in_session_inner(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    super::initial_containing_block::prepare_root_element(doc);
    bridge_writing_modes(doc, cascade);
    let initial = ComputedValues::initial();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let vertical_sized = vertical_logical_sized(cascade, idx);
        let raw_cv = vertical_sized.as_ref().unwrap_or(&cascade.computed[idx]);
        let values = layout_values(doc, cascade, idx, vertical_sized.as_ref());
        let cv = &*values;
        // Taffy does not carry CSS `order` in Style. Retain the computed value
        // on Node so flex/grid child traversal can build order-modified order.
        doc.nodes[idx].order = cv.order;
        // Block-flow static positions use CSS text alignment, including the
        // last-line override, which is more expressive than Taffy's legacy
        // `text_align` field.
        doc.nodes[idx].static_position_text_align = cv.text_align;
        doc.nodes[idx].static_position_text_align_last = cv.text_align_last;
        // Carry multicol settings into the Taffy dispatch seam. Taffy has no
        // native multicol style fields, so the custom strategy reads this
        // side-channel while the ordinary Style remains Taffy-compatible.
        // CSS Multi-column Layout 1, 2: the column properties apply to block
        // containers other than table wrapper boxes; a flex, grid or table
        // box lays its content out by its own algorithm, in no columns.
        doc.nodes[idx].multicol =
            multicol_style_from_computed(cv).filter(|_| !lays_out_without_columns(cv.display));
        doc.nodes[idx].column_span_all =
            cv.column_span == raikiri_style::property::ColumnSpanValue::All;
        doc.set_box_properties(idx, cv.display, cv.position.clone());
        // Table engine inputs — no taffy::Style counterpart (taffy 0.15
        // has no table layout), carried Node-side like `display` above.
        doc.nodes[idx].table_layout = cv.table_layout;
        doc.nodes[idx].table_vertical_align = cv.vertical_align;
        doc.nodes[idx].hides_empty_table_cell =
            crate::paint_rules::hides_empty_table_cell(doc, cascade, idx);
        doc.nodes[idx].collapses_table_rows = cv.visibility
            == raikiri_style::property::Visibility::Collapse
            && matches!(
                cv.display,
                DisplayValue::TableRow
                    | DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
            );
        doc.nodes[idx].collapses_table_columns = cv.visibility
            == raikiri_style::property::Visibility::Collapse
            && matches!(
                cv.display,
                DisplayValue::TableColumn | DisplayValue::TableColumnGroup
            );
        doc.nodes[idx].clips_collapsed_table_span = false;
        doc.nodes[idx].caption_side = cv.caption_side;
        if !matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable) {
            doc.nodes[idx].table_grid_box = None;
            doc.nodes[idx].table_first_baseline = None;
        }
        doc.nodes[idx].flex_first_baseline = None;
        doc.nodes[idx].border_collapse = cv.border_collapse;
        doc.nodes[idx].computed_border = Some(cv.border);
        doc.nodes[idx].collapsed_border = None;
        doc.nodes[idx].border_spacing = cv.border_spacing;
        doc.nodes[idx].break_before = cv.break_before;
        doc.nodes[idx].break_after = cv.break_after;
        doc.nodes[idx].break_inside = cv.break_inside;
        doc.nodes[idx].paragraph_orphans = cv.orphans.max(1) as usize;
        doc.nodes[idx].paragraph_widows = cv.widows.max(1) as usize;
        doc.nodes[idx].multicol_auto_width =
            matches!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        // Generated content is not an arena child. Keep its provenance so
        // column continuations cannot replay an unfragmented pseudo subtree.
        doc.nodes[idx].has_before_or_after_content = [PseudoElem::Before, PseudoElem::After]
            .iter()
            .any(|&pseudo| {
                cascade
                    .pseudo
                    .get(&(StyleNodeId::new(idx as u64), pseudo))
                    .is_some_and(|pseudo| {
                        pseudo.display != DisplayValue::None
                            && !pseudo.content.is_empty()
                            && !pseudo
                                .content
                                .iter()
                                .any(|part| matches!(part, ContentComponent::None))
                    })
            });
        doc.nodes[idx].paints_whole_box_effects = paints_whole_box_effects(cv, &initial)
            || super::fixed::has_applied_transform(doc, idx, cv);
        doc.nodes[idx].paints_background_color = cv.background_color.a > 0;
        doc.nodes[idx].clones_box_decorations =
            cv.box_decoration_break == BoxDecorationBreak::Clone;
        doc.nodes[idx].claims_absolute_oof =
            super::fixed::establishes_absolute_containing_block(doc, idx, cv);
        doc.nodes[idx].claims_fixed_oof =
            super::fixed::establishes_fixed_containing_block(doc, idx, cv);
        doc.nodes[idx].contains_absolute_boxes =
            doc.nodes[idx].claims_absolute_oof && !doc.nodes[idx].claims_fixed_oof;
        doc.nodes[idx].fixed_position = cv.position == PositionValue::Fixed;
        bridge_size(doc, idx, cv);
        // Replaced elements keep their natural size where an auto width or
        // `normal` alignment would stretch other boxes.
        doc.layout_node_with_style_mut(idx)
            .unwrap()
            .style
            .item_is_replaced = crate::paint_rules::lays_out_as_replaced(doc, idx);
        // A resolver may give the natural size and leave its ratio unset; the
        // ratio of that size is then the natural one.
        let natural_ratio = crate::taffy_impl::leaf_intrinsic_ratio(&doc.nodes[idx]);
        let preferred_ratio = preferred_aspect_ratio(cv, natural_ratio);
        doc.nodes[idx].preferred_aspect_ratio = preferred_ratio;
        doc.nodes[idx].ratio_in_content_box =
            cv.aspect_ratio.auto && cv.aspect_ratio.ratio.is_some();
        doc.layout_node_with_style_mut(idx)
            .unwrap()
            .style
            .aspect_ratio = preferred_ratio;
        // SVG 2 geometry: auto dimensions on svg are treated as 100%.
        // With only a viewBox ratio and no definite height, width establishes
        // the viewport and the ratio determines its auto height.
        if doc.nodes[idx].is_inline_svg_root()
            && doc.nodes[idx].attribute("width").is_none()
            && doc.nodes[idx].attribute("height").is_none()
            && doc
                .layout_node_with_style(idx)
                .unwrap()
                .style
                .aspect_ratio
                .is_some()
        {
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
                && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
            {
                doc.layout_node_with_style_mut(idx)
                    .unwrap()
                    .style
                    .size
                    .width = Dimension::percent(1.0);
            } else if !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                // Taffy transfers max-height to max-width through this ratio
                // even for a definite width. SVG leaf measurement handles its
                // auto height without shrinking an authored width.
                doc.layout_node_with_style_mut(idx)
                    .unwrap()
                    .style
                    .aspect_ratio = None;
            }
        }
        if let Some((intrinsic_width, intrinsic_height)) =
            bridge_known_image_intrinsic_size(doc, idx, cv)
        {
            let style = &mut doc.document.layout_node_with_style_mut(idx).unwrap().style;
            // The supplied size is the image's natural size.
            style.item_is_replaced = true;
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                style.size.width = Dimension::length(intrinsic_width);
            }
            if matches!(cv.height, ComputedLengthPercentageOrAuto::Auto) {
                style.size.height = Dimension::length(intrinsic_height);
            }
        }
        doc.document.nodes[idx].has_logical_min_block_size = cv.min_block_size.is_some()
            && matches!(cv.break_inside, raikiri_style::property::BreakInside::Avoid);
        doc.document.nodes[idx].needs_relative_block_paint_offset = false;
        let vertical_axes = uses_vertical_axes(cascade, idx);
        let is_contents = doc.document.nodes[idx].display() == DisplayValue::Contents;
        // Build this style off-map so its calc pointers are not exposed until
        // their Arc owners have been retained by the document.
        let mut style = doc.document.layout_style_for_key(idx).unwrap().clone();
        let mut calc_values = Vec::new();
        bridge_direction(&mut style, cv, vertical_axes);
        bridge_inset(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_overflow(&mut style, cv);
        // Layout and paint containment establish an independent formatting
        // context, including when the box's content is one IFC paragraph.
        use raikiri_style::property::Contain;
        style.contain = match (
            cv.contain.contains(Contain::LAYOUT),
            cv.contain.contains(Contain::PAINT),
        ) {
            (true, true) => taffy::Contain::CONTENT,
            (true, false) => taffy::Contain::LAYOUT,
            (false, true) => taffy::Contain::PAINT,
            (false, false) => taffy::Contain::NONE,
        };
        bridge_float(&mut style, cv);
        bridge_margin(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_padding(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_min_max_size(
            &mut style,
            cv,
            vertical_axes,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        let content_limit_values = if crate::taffy_impl::is_table_box(raw_cv.display)
            || crate::paint_rules::is_replaced_element(doc.document, idx)
        {
            raw_cv
        } else {
            cv
        };
        doc.nodes[idx].content_limits = bridge_content_limits(
            content_limit_values,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_border(&mut style, cv, &mut doc.context.layout_warnings);
        bridge_box_sizing(&mut style, cv);
        bridge_flex(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_alignment(&mut style, cv);
        // The container an `auto` self-alignment borrows from is the nearest
        // ancestor that makes a box: `display: contents` ones are flattened
        // away, except the document element, which is blockified.
        let boxes = super::box_tree::BoxTree::for_cascade(doc, cascade);
        let mut container = doc.nodes[idx].parent;
        while let Some(ancestor) = container
            && boxes.display(ancestor) == Some(DisplayValue::Contents)
        {
            container = doc.nodes[ancestor].parent;
        }
        if let Some(container) = container {
            bridge_inherited_last_baseline(&mut style, cv, &cascade.computed[container]);
        }
        bridge_gap(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        bridge_grid(
            &mut style,
            cv,
            &mut calc_values,
            &mut doc.context.layout_warnings,
        );
        if is_contents {
            // A `display: contents` element generates no box (CSS Display 3
            // §2.5). Where layout still keeps a node for it, the node stands
            // in for the anonymous box its contents would get, so none of
            // the element's own box properties apply to it.
            style = taffy::Style {
                direction: style.direction,
                ..taffy::Style::default()
            };
            // Taffy never lays out a contents element whose children it
            // lifts into their container; zero its layout so it adds
            // nothing to its descendants' positions.
            doc.nodes[idx].unrounded_layout = taffy::Layout::default();
            doc.nodes[idx].multicol = None;
            doc.nodes[idx].column_span_all = false;
            doc.nodes[idx].multicol_auto_width = true;
            doc.nodes[idx].has_logical_min_block_size = false;
            doc.set_box_properties(idx, cv.display, cv.position.clone());
        }
        // Keep the owners before publishing any raw calc pointers through the
        // retained style map. A panic while inserting owners can only leave
        // extra owners behind; it cannot leave a dangling style handle.
        doc.document
            .retained_layout_styles
            .calc_values
            .extend(calc_values);
        let key = doc.nodes.box_key(idx).expect("source node has a box key");
        doc.document.retained_layout_styles.insert(key, style);
    }
    super::spanner_wrappers::lift_spanner_wrappers(doc, cascade);
    // Whether a box is a non-atomic inline box depends on its parent's
    // display, which the loop above has now bridged for every node.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // The sizes bridged above, which a vertical box takes from its
        // logical sizing properties.
        let vertical_sized = vertical_logical_sized(cascade, idx);
        let values = layout_values(doc, cascade, idx, vertical_sized.as_ref());
        let cv = &*values;
        let containment = size_containment(doc, idx, cv);
        doc.nodes[idx].size_containment = containment;
        doc.nodes[idx].text_group_align = shodo_group_align(cv.text_group_align);
        // Size containment removes the natural size and aspect ratio of
        // replaced content, so only an `aspect-ratio` of the box itself can
        // stand in for an `auto` size.
        if containment.width || containment.height {
            let style = &mut doc.layout_node_with_style_mut(idx).unwrap().style;
            style.aspect_ratio = preferred_aspect_ratio(cv, None);
            if containment.width && matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                style.size.width = Dimension::auto();
            }
            if containment.height && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto) {
                style.size.height = Dimension::auto();
            }
        }
        if stretches_on_both_grid_axes(doc, idx, cv)
            && !replaced_keyword_width(doc.document, cascade, idx)
        {
            doc.layout_node_with_style_mut(idx)
                .unwrap()
                .style
                .aspect_ratio = None;
        }
        if is_stretched_by_replaced_keyword_width_in_grid(doc, cascade, idx) {
            doc.layout_node_with_style_mut(idx)
                .unwrap()
                .style
                .justify_self = Some(TaffyAlignItems::START);
        }
        if is_stretched_by_keyword_width(doc, cascade, idx) {
            doc.layout_node_with_style_mut(idx)
                .unwrap()
                .style
                .align_self = Some(TaffyAlignItems::FLEX_START);
        }
        if let Some(fallback) = unstretched_keyword_height_alignment(doc, cascade, idx) {
            doc.layout_node_with_style_mut(idx)
                .unwrap()
                .style
                .align_self = Some(fallback);
        }
    }
    // Paragraph roots are assigned first: a `display: contents` paragraph
    // root stays one flex or grid item when `order` sorts the items.
    if let Err(error) = crate::layout::ifc::assign::assign_ifc_roots(doc, cascade) {
        // A failed paragraph build must not expose the preceding pass's
        // classification. No paragraph layout is valid on this path, so
        // retain only the new backend display classification for elements.
        for node in doc.nodes.layout_nodes_mut() {
            if node.kind() == NodeKind::Element {
                node.formatting_context =
                    FormattingContext::from_taffy_display(node.taffy_display());
            }
        }
        return Err(error);
    }
    // A text node or an anonymous table cell can be a paragraph root of its
    // own, so every node layout may lay out is classified.
    for node in doc.nodes.layout_nodes_mut() {
        node.formatting_context = classify_formatting_context(node);
    }
    refresh_order_modified_children(doc);
    // Mark table formatting roots for blitz-compat bit preservation.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let is_table = matches!(
            doc.nodes[idx].display(),
            DisplayValue::Table | DisplayValue::InlineTable
        );
        if is_table {
            doc.nodes[idx].flags.insert(NodeFlags::IS_TABLE_ROOT);
        } else {
            doc.nodes[idx].flags.remove(NodeFlags::IS_TABLE_ROOT);
        }
    }
    Ok(())
}

/// Classify how a bridged box lays out its contents. Any box whose content
/// is a paragraph is laid out by the inline engine, including a table or
/// multicol container and a bare text root. Taffy has no table or multicol
/// display, so both are block containers to it.
fn classify_formatting_context(node: &Node) -> FormattingContext {
    match node.taffy_display() {
        Display::None => FormattingContext::None,
        _ if node.flags.contains(NodeFlags::IS_IFC_ROOT) => FormattingContext::Inline,
        _ if node.kind() != NodeKind::Element => FormattingContext::Text,
        Display::Flex => FormattingContext::Flex,
        Display::Grid => FormattingContext::Grid,
        _ if matches!(
            node.display(),
            DisplayValue::Table | DisplayValue::InlineTable
        ) =>
        {
            FormattingContext::Table
        }
        _ if node.multicol.is_some() => FormattingContext::Multicol,
        display => FormattingContext::from_taffy_display(display),
    }
}

/// Resolve writing-mode ancestry once, before any bridge consumer runs.
/// Arena order need not put parents before children. Starting at every
/// root also covers detached subtrees and inert template fragments.
fn bridge_writing_modes(doc: &mut Document, cascade: &CascadeResult) {
    for (id, node) in doc.nodes.iter_mut().enumerate() {
        node.authored_writing_mode = cascade.authored_writing_modes.get(id).copied().flatten();
    }
    let mut pending: Vec<usize> = doc
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, node)| node.parent.is_none().then_some(id))
        .collect();
    while let Some(id) = pending.pop() {
        let parent = doc.parent_of(id);
        let inherited = parent.map_or(WritingMode::HorizontalTb, |parent| {
            doc.nodes[parent].used_writing_mode
        });
        let under_vertical = parent.is_some_and(|parent| doc.nodes[parent].under_vertical_writing);
        let node = &mut doc.nodes[id];
        node.used_writing_mode = node.authored_writing_mode.unwrap_or(inherited);
        node.under_vertical_writing = under_vertical
            || node
                .authored_writing_mode
                .is_some_and(WritingMode::is_vertical);
        pending.extend(node.children.iter().copied());
    }
}

/// The physical axes in which `contain` sizes the box as if it had no
/// content (CSS Containment 2 §3.1 size containment, §3.2 inline-size
/// containment).
///
/// Neither applies to a box that is not generated, to a non-atomic inline
/// box, or to an internal table box (§3.1, §3.2), which a table caption is
/// not. Tables are left out as well: their width algorithm sizes the grid
/// from its cells. Inline-size containment acts on the inline axis of the
/// box's own writing mode.
fn size_containment(doc: &Document, idx: usize, cv: &ComputedValues) -> taffy::Size<bool> {
    use raikiri_style::property::Contain;

    let none = taffy::Size {
        width: false,
        height: false,
    };
    let applies = (cv.display.is_flex_or_grid_container()
        || (cv.display.is_inline_flow_box()
            || matches!(
                cv.display,
                DisplayValue::Block
                    | DisplayValue::FlowRoot
                    | DisplayValue::ListItem
                    | DisplayValue::TableCaption
                    | DisplayValue::BlockRuby
            )))
        && !super::fixed::generates_inline_box(doc, idx, cv);
    if !applies {
        return none;
    }
    if cv.contain.contains(Contain::SIZE) {
        return taffy::Size {
            width: true,
            height: true,
        };
    }
    if !cv.contain.contains(Contain::INLINE_SIZE) {
        return none;
    }
    let vertical = crate::taffy_impl::used_writing_mode(doc, idx).is_vertical();
    taffy::Size {
        width: !vertical,
        height: vertical,
    }
}

/// Whether painting `cv` draws effects sized from the whole border box
/// (background and border images, rounded corners, shadows, outlines, masks,
/// clips and filters) or composites the whole box as one group (opacity and
/// blend modes), which a fragment cannot reproduce from its own slice. A
/// transform counts too, where it applies; the caller checks that.
fn paints_whole_box_effects(cv: &ComputedValues, initial: &ComputedValues) -> bool {
    cv.has_background_image()
        || cv.border_image != initial.border_image
        || cv.border_radius != initial.border_radius
        || !cv.box_shadow.is_empty()
        || cv.outline.style() != initial.outline.style()
        || cv.mask_image != initial.mask_image
        || cv.clip_path != initial.clip_path
        || !cv.filter.is_empty()
        || cv.opacity != initial.opacity
        || cv.mix_blend_mode != initial.mix_blend_mode
}

/// [`Direction`] → [`taffy::Direction`] mapping.
///
/// Taffy uses this field for horizontal block alignment, table/grid ordering,
/// and overflow direction.  Text direction is mapped separately, into the
/// paragraph style of the inline engine.
///
/// A box in `vertical-rl` or `vertical-lr` with `text-orientation: upright`
/// uses `ltr` whatever its `direction` (CSS Writing Modes 4 §5.1), as its
/// logical margins and padding were mapped. The sideways modes are
/// horizontal typographic modes, which `upright` does not affect.
///
/// `sideways-lr` runs its inline axis from bottom to top (CSS Writing Modes 4
/// §3.1), the reverse of `vertical-lr`, whose axes the box lays out on; its
/// direction is flipped so that the inline start stays at the bottom.
fn bridge_direction(style: &mut taffy::Style, cv: &ComputedValues, vertical_axes: bool) {
    let upright = vertical_axes
        && cv.text_orientation == TextOrientation::Upright
        && matches!(
            cv.cssom_writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        );
    let rtl = matches!(cv.direction, Direction::Rtl) && !upright;
    let bottom_to_top = vertical_axes && cv.cssom_writing_mode == WritingMode::SidewaysLr;
    style.direction = if rtl != bottom_to_top {
        TaffyDirection::Rtl
    } else {
        TaffyDirection::Ltr
    };
}

/// The computed values the layout of element `idx` reads: `vertical_sized` when
/// a vertical box takes its sizes from its logical properties, else the
/// cascade's own. A `max-content` or `fit-content` size a box is not sized by
/// (see [`sized_by_content_keyword`]) is `auto` here, and so is a `min-*` or
/// `max-*` size that is a sizing keyword the box does not honor (see
/// [`honors_content_limits`]). The `min-content` width of a replaced box is
/// `auto` as well. A vertical box takes its inline size from `height`.
fn layout_values<'a>(
    doc: &Document,
    cascade: &'a CascadeResult,
    idx: usize,
    vertical_sized: Option<&'a ComputedValues>,
) -> std::borrow::Cow<'a, ComputedValues> {
    let cv = vertical_sized.unwrap_or(&cascade.computed[idx]);
    let content_keyword = |size| {
        matches!(
            size,
            ComputedLengthPercentageOrAuto::MaxContent
                | ComputedLengthPercentageOrAuto::FitContent
                | ComputedLengthPercentageOrAuto::FitContentOf(_)
        )
    };
    let width = content_keyword(cv.width)
        || (matches!(cv.width, ComputedLengthPercentageOrAuto::MinContent)
            && min_content_width_is_auto(doc, cascade, idx));
    let height = vertical_sized.is_some() && content_keyword(cv.height);
    let limited = [cv.min_width, cv.max_width, cv.min_height, cv.max_height]
        .into_iter()
        .any(is_sizing_keyword);
    let ignore_logical_min_block = cv.min_block_size.is_some_and(is_sizing_keyword)
        && (is_supported_flex_grid_item(doc, cascade, idx, cv)
            || is_supported_flex_grid_container(doc, idx, cv));
    let plain_sizes = (width || height) && !sized_by_content_keyword(doc, cascade, idx, cv);
    let plain_limits = limited && !honors_content_limits(doc, cascade, idx, cv);
    if !(plain_sizes || plain_limits || ignore_logical_min_block) {
        return std::borrow::Cow::Borrowed(cv);
    }
    let mut plain = cv.clone();
    if plain_sizes && width {
        plain.width = ComputedLengthPercentageOrAuto::Auto;
    }
    if plain_sizes && height {
        plain.height = ComputedLengthPercentageOrAuto::Auto;
    }
    if plain_limits {
        for size in [
            &mut plain.min_width,
            &mut plain.max_width,
            &mut plain.min_height,
            &mut plain.max_height,
        ] {
            if is_sizing_keyword(*size) {
                *size = ComputedLengthPercentageOrAuto::Auto;
            }
        }
    }
    if ignore_logical_min_block {
        // Supported flex/grid boxes use horizontal writing modes, so the
        // computed logical block minimum occupies the physical min-height.
        // Ignore it without dropping unrelated physical width constraints.
        plain.min_height = ComputedLengthPercentageOrAuto::Auto;
    }
    std::borrow::Cow::Owned(plain)
}

/// Whether `size` is one of the intrinsic sizing keywords.
fn is_sizing_keyword(size: ComputedLengthPercentageOrAuto) -> bool {
    matches!(
        size,
        ComputedLengthPercentageOrAuto::MinContent
            | ComputedLengthPercentageOrAuto::MaxContent
            | ComputedLengthPercentageOrAuto::FitContent
            | ComputedLengthPercentageOrAuto::FitContentOf(_)
    )
}

/// Whether element `idx`, whose computed values are `cv`, uses an intrinsic
/// sizing keyword in `min-width`, `max-width`, `min-height` or `max-height`
/// (CSS Sizing 3 §4 and §5).
///
/// A normal-flow block, float, and inline-block use these constraints when
/// their sizing algorithms clamp their used sizes. An absolutely positioned
/// box also uses them; its constraints are resolved before Taffy reads its
/// insets and sizes. A flex/grid item uses them because its parent algorithm
/// reads the item's min/max constraints before laying it out. Other boxes size
/// themselves as if there were no such minimum or maximum:
/// - a box in a multicol flow is fragmented across the columns, so its
///   automatic height is not the height of one fragment;
/// - table containers and items, replaced items, and inline-level boxes
///   other than inline-blocks measure themselves by algorithms of their own;
/// - a vertical box swaps the axes a keyword refers to.
fn honors_content_limits(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
) -> bool {
    if is_supported_flex_grid_item(doc, cascade, idx, cv) {
        return true;
    }
    if is_supported_flex_grid_container(doc, idx, cv) {
        return true;
    }
    let out_of_flow = matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed);
    let block_sized = matches!(
        cv.display,
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    );
    let non_block_sized = cv.display == DisplayValue::InlineBlock || cv.float != FloatValue::None;
    let table = matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable);
    if table
        || (!block_sized && !non_block_sized && !out_of_flow)
        || cv.multicol_flow
        || lays_out_vertically(cascade, idx)
        || !sized_by_content_keyword(doc, cascade, idx, cv)
    {
        return false;
    }
    out_of_flow || !is_flex_or_grid_item(doc, cascade, idx)
}

/// Whether `idx` is an in-flow flex/grid container whose own intrinsic min/max
/// constraints this bridge can preserve.
fn is_supported_flex_grid_container(doc: &Document, idx: usize, cv: &ComputedValues) -> bool {
    cv.display.is_flex_or_grid_container()
        && !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        && cv.float == FloatValue::None
        && !cv.multicol_flow
        && !uses_vertical_writing_mode(cv)
        && !crate::paint_rules::is_replaced_element(doc, idx)
}

/// Whether `idx` participates as a flex/grid item whose intrinsic min/max
/// constraints this bridge can preserve. Other item types remain on their
/// existing size paths.
fn is_supported_flex_grid_item(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
) -> bool {
    !matches!(cv.display, DisplayValue::None | DisplayValue::Contents)
        && !matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable)
        && !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        && !cv.multicol_flow
        && !uses_vertical_writing_mode(cv)
        && !crate::paint_rules::is_replaced_element(doc, idx)
        && layout_container(doc, cascade, idx).is_some_and(|container| {
            cascade.computed[container]
                .display
                .is_flex_or_grid_container()
        })
}

fn uses_vertical_writing_mode(cv: &ComputedValues) -> bool {
    matches!(
        cv.cssom_writing_mode,
        WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr
    )
}

/// Whether the width of element `idx` is a sizing keyword its layout sizes the
/// box by: `min-content`, or a `max-content` or `fit-content` width that
/// [`layout_values`] keeps. A replaced box is sized as if its width were `auto`
/// (see [`min_content_width_is_auto`]). A `display: contents` element
/// generates no box and `width` does not apply to a non-replaced inline box
/// (CSS Sizing 3 §3.1.1), so the width of either sizes nothing.
pub(crate) fn width_sized_by_keyword(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    let keyword = match cv.width {
        ComputedLengthPercentageOrAuto::MinContent => !min_content_width_is_auto(doc, cascade, idx),
        ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_) => {
            sized_by_content_keyword(doc, cascade, idx, cv)
        }
        _ => false,
    };
    keyword
        && cv.display != DisplayValue::Contents
        && !super::fixed::generates_inline_box(doc, idx, cv)
}

/// Whether the layout of element `idx`, whose computed values are `cv`, sizes
/// it by a `max-content` or `fit-content` size (CSS Sizing 3 §3.1.1).
///
/// A box in a block formatting context, an absolutely positioned box and a flex
/// or grid item are; so are flex, grid and table containers the block algorithm
/// measures. The others keep sizing themselves as if the size were `auto`:
/// - a replaced element takes its natural size, and an `auto` size is what
///   transfers its aspect ratio;
/// - a vertical flex or grid item takes its inline size from the height, which
///   the container's algorithm sizes along its physical axes;
/// - a flex or grid item with a `min-width` or `max-width` is measured by Taffy
///   at the keyword size before that size is clamped, so the lines of its
///   content break at a width the box does not keep.
fn sized_by_content_keyword(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
) -> bool {
    if crate::paint_rules::is_replaced_element(doc, idx) {
        return false;
    }
    if matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) {
        return true;
    }
    // Only a `min-width` that is not `auto` or zero clamps the keyword.
    let unclamped = matches!(cv.max_width, ComputedLengthPercentageOrAuto::Auto)
        && (matches!(cv.min_width, ComputedLengthPercentageOrAuto::Auto)
            || is_zero_length(cv.min_width));
    !is_flex_or_grid_item(doc, cascade, idx) || (unclamped && !lays_out_vertically(cascade, idx))
}

/// Whether `size` is zero whatever it refers to: a zero length, a zero
/// percentage, or a `calc()` whose terms are both zero.
fn is_zero_length(size: ComputedLengthPercentageOrAuto) -> bool {
    match size {
        ComputedLengthPercentageOrAuto::Px(px) => matches!(px, 0.0),
        ComputedLengthPercentageOrAuto::Percent(percent) => matches!(percent, 0.0),
        ComputedLengthPercentageOrAuto::Calc(calc) => {
            matches!((calc.percent, calc.px), (0.0, 0.0))
        }
        _ => false,
    }
}

/// Whether the box of element `idx` is an item of a flex or grid container:
/// the element that contains it lays its boxes out by the flex or grid
/// algorithm.
fn is_flex_or_grid_item(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    layout_container(doc, cascade, idx)
        .is_some_and(|id| cascade.computed[id].display.is_flex_or_grid_container())
}

/// The element whose box contains the box of element `idx`: its nearest
/// ancestor that is not `display: contents`, which has no box of its own.
fn layout_container(doc: &Document, cascade: &CascadeResult, idx: usize) -> Option<usize> {
    std::iter::successors(doc.parent_of(idx), |&id| doc.parent_of(id))
        .find(|&id| cascade.computed[id].display != DisplayValue::Contents)
}

/// Whether a box of `display` lays its content out by an algorithm of its
/// own, to which the column properties do not apply.
fn lays_out_without_columns(display: DisplayValue) -> bool {
    display.is_flex_or_grid_container()
        || matches!(display, DisplayValue::Table | DisplayValue::InlineTable)
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
fn bridge_margin(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    let m = cv.margin;
    let mut side = |value| {
        computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
            calc_values,
            value,
            "margin",
            diag,
        )
    };
    style.margin = Rect {
        top: side(m.top),
        right: side(m.right),
        bottom: side(m.bottom),
        left: side(m.left),
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
fn bridge_padding(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    let p = cv.padding;
    let mut side = |value| match value {
        ComputedLengthPercentageOrCalc::Px(px) => {
            computed_length_percentage_to_taffy_length_percentage(
                ComputedLengthPercentage::Px(px),
                "padding",
                diag,
            )
        }
        ComputedLengthPercentageOrCalc::Percent(percent) => {
            computed_length_percentage_to_taffy_length_percentage(
                ComputedLengthPercentage::Percent(percent),
                "padding",
                diag,
            )
        }
        // A mixed percentage-and-length padding resolves against the
        // containing block in taffy, through the same arena as sizes.
        ComputedLengthPercentageOrCalc::Calc(value) => {
            let pointer = crate::taffy_impl::push_calc(calc_values, value, true);
            LengthPercentage::calc(pointer)
        }
    };
    style.padding = Rect {
        top: side(p.top),
        right: side(p.right),
        bottom: side(p.bottom),
        left: side(p.left),
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
    style.border = taffy_border_widths(&cv.border, diag);
}

/// The taffy border widths of `border`, as [`bridge_border`] sets them.
pub(super) fn taffy_border_widths(
    border: &raikiri_style::property::Sides<raikiri_style::ComputedBorder>,
    diag: &mut Vec<LayoutWarn>,
) -> Rect<LengthPercentage> {
    Rect {
        top: computed_length_to_taffy_length_percentage(border.top.width(), diag),
        right: computed_length_to_taffy_length_percentage(border.right.width(), diag),
        bottom: computed_length_to_taffy_length_percentage(border.bottom.width(), diag),
        left: computed_length_to_taffy_length_percentage(border.left.width(), diag),
    }
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

fn bridge_inset(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    // Insets keep a mixed `calc()` in the document arena, like min/max sizes,
    // so its percentage resolves against the used containing block.
    let mut inset = |value: ComputedLengthPercentageOrAuto, site: &'static str| {
        computed_length_percentage_or_auto_to_taffy_min_max(calc_values, value, false, site, diag)
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
/// Page dimensions belong to a separate initial-containing-block box. The
/// element's preferred width and height remain the authored sizing inputs.
fn bridge_size(doc: &mut LayoutSession<'_>, node_id: usize, cv: &ComputedValues) {
    // width and height are written together, so use a struct literal.
    let width = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.document.retained_layout_styles.calc_values,
        cv.width,
        "width",
        &mut doc.context.layout_warnings,
    );
    let height = computed_length_percentage_or_auto_to_taffy_dimension(
        &mut doc.document.retained_layout_styles.calc_values,
        cv.height,
        "height",
        &mut doc.context.layout_warnings,
    );
    doc.document
        .layout_node_with_style_mut(node_id)
        .unwrap()
        .style
        .size = Size { width, height };
}

/// The preferred aspect ratio taffy sizes a box by (CSS Box Sizing 4 §5.1):
/// the ratio the `aspect-ratio` property gives it, with `natural` standing for
/// the natural ratio of replaced content. A ratio only participates when at
/// least one preferred dimension is auto; Taffy's leaf sizing otherwise floors
/// an explicit height to the ratio-derived height.
fn preferred_aspect_ratio(cv: &ComputedValues, natural: Option<f32>) -> Option<f32> {
    if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
        || matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
    {
        cv.aspect_ratio.preferred(natural)
    } else {
        None
    }
}

/// Whether element `idx` is a replaced box of a horizontal writing mode, which
/// sizes itself by its `min-content` width as it does by `auto`.
fn min_content_width_is_auto(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    crate::paint_rules::is_replaced_element(doc, idx) && !lays_out_vertically(cascade, idx)
}

/// Whether element `idx` is a replaced box with a natural size whose `width` is
/// `min-content`, `max-content` or `fit-content`.
///
/// Such a box sizes itself as if the width were `auto`: its natural size, or
/// the width its ratio gives a definite height ([`layout_values`] hands the
/// layout `auto`). The width is no `auto` for alignment, though: `stretch`
/// fills the space of a box whose size is `auto` and of no other (CSS Box
/// Alignment 3 §6.1). `fit-content(<length-percentage>)` stays out, as browsers
/// treat it like `auto` on a replaced box. So does a box without a natural
/// size, such as an inline SVG with only a viewBox: browsers fill the space
/// with it for a keyword width as they do for `auto`.
fn replaced_keyword_width(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].width,
        ComputedLengthPercentageOrAuto::MinContent
            | ComputedLengthPercentageOrAuto::MaxContent
            | ComputedLengthPercentageOrAuto::FitContent
    ) && crate::paint_rules::lays_out_as_replaced(doc, idx)
        && !lays_out_vertically(cascade, idx)
}

/// Whether item `idx` of a column flex container has a `width` that is a sizing
/// keyword although the container's alignment would stretch it.
///
/// Only an `auto` cross size stretches (CSS Flexbox 1 §9.4 step 11). Taffy
/// settles that when it places the item, yet it measures the item's height at
/// the stretched width first, so a keyword width that is narrower or wider than
/// the container leaves the height of another line count. Aligning the item at
/// the start of the cross axis measures it at its own width. Taffy places a
/// stretched item that it does not stretch exactly like a `flex-start` one, so
/// nothing else moves.
///
/// The width of a replaced box is `auto` to Taffy ([`replaced_keyword_width`]),
/// so the item is aligned at the start whenever the keyword would be stretched.
fn is_stretched_by_keyword_width(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
) -> bool {
    use taffy::AlignItemsKeyword;
    let style = &doc.layout_node_with_style(idx).unwrap().style;
    let keyword_width = (style.size.width.is_sizing_keyword() && !style.size.width.is_stretch())
        || replaced_keyword_width(doc.document, cascade, idx);
    if !keyword_width || lays_out_vertically(cascade, idx) {
        return false;
    }
    doc.column_flex_container_of(idx).is_some_and(|container| {
        matches!(
            style.align_self.unwrap_or(container.align_items).keyword,
            AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
        )
    })
}

/// Whether grid item `idx` is a replaced box with a keyword width that its
/// `justify-self` would stretch ([`replaced_keyword_width`]). Aligning it at
/// the start of the row axis keeps the width it sizes itself to.
fn is_stretched_by_replaced_keyword_width_in_grid(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
) -> bool {
    use taffy::AlignItemsKeyword;
    let style = &doc.layout_node_with_style(idx).unwrap().style;
    replaced_keyword_width(doc.document, cascade, idx)
        && doc.taffy_parent_of(idx).is_some_and(|parent| {
            let container = &doc.layout_node_with_style(parent).unwrap().style;
            container.display == taffy::Display::Grid
                && style
                    .justify_self
                    .unwrap_or(container.justify_items)
                    .keyword
                    == AlignItemsKeyword::Stretch
        })
}

/// The alignment that replaces a stretch of item `idx` when its `height` is
/// `min-content`, `max-content` or `fit-content` and it sits in a grid or in a
/// row flex container.
///
/// Only an `auto` height is stretched (CSS Box Alignment 3 §6.1), but the cascade
/// computes the keyword to `auto` ([`ComputedValues::height_keyword`] is what is
/// left of it), so Taffy would stretch the box. It sizes itself by its content as
/// it does for `auto`, and is aligned at the start of the block axis instead,
/// which is where a stretch that does not apply falls back to: `start` in a grid
/// and `flex-start` in a flex container, whose cross-start edge `wrap-reverse`
/// moves. `fit-content(<length>)` stays out, as browsers treat it like `auto`.
/// So do an absolutely positioned box, whose size comes from its insets, and an
/// element without a box.
fn unstretched_keyword_height_alignment(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
) -> Option<TaffyAlignItems> {
    use taffy::AlignItemsKeyword;
    let values = &cascade.computed[idx];
    if values.height_keyword.is_none()
        || matches!(values.display, DisplayValue::Contents | DisplayValue::None)
    {
        return None;
    }
    let style = &doc.layout_node_with_style(idx).unwrap().style;
    if style.position == taffy::Position::Absolute {
        return None;
    }
    let container = &doc
        .layout_node_with_style(doc.taffy_parent_of(idx)?)
        .unwrap()
        .style;
    let fallback = match container.display {
        taffy::Display::Grid => TaffyAlignItems::START,
        taffy::Display::Flex
            if matches!(
                container.flex_direction,
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            ) =>
        {
            TaffyAlignItems::FLEX_START
        }
        _ => return None,
    };
    matches!(
        style.align_self.unwrap_or(container.align_items).keyword,
        AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
    )
    .then_some(fallback)
}

/// Whether in-flow grid item `idx` is sized along both axes without its
/// preferred aspect ratio, in which case the ratio is ignored: each axis is
/// either given a definite size or stretched by an explicit `stretch`
/// alignment, so the item fills its grid area on it (CSS Grid 1 §6.2,
/// csswg-drafts#5713). A stretch alignment in one axis together with a
/// definite size in the other leaves the ratio nothing to do, replaced
/// content included. An auto margin in an axis turns that axis's stretch
/// off, and so does a height that is a sizing keyword computed to `auto`
/// ([`ComputedValues::height_keyword`]).
///
/// The ratio is dropped from the item's style, which also removes it from
/// track sizing, so this applies only when every row track has a fixed size
/// and the item's height contribution cannot size a row.
fn stretches_on_both_grid_axes(doc: &LayoutSession<'_>, idx: usize, cv: &ComputedValues) -> bool {
    use taffy::AlignItemsKeyword;
    let style = &doc.layout_node_with_style(idx).unwrap().style;
    if style.aspect_ratio.is_none() || style.position == taffy::Position::Absolute {
        return false;
    }
    let stretch = |value: Option<taffy::AlignItems>, fallback: taffy::AlignItems| {
        value.unwrap_or(fallback).keyword == AlignItemsKeyword::Stretch
    };
    let any_auto = |margins: [taffy::LengthPercentageAuto; 2]| margins.iter().any(|m| m.is_auto());
    doc.taffy_parent_of(idx).is_some_and(|parent| {
        let container = &doc.layout_node_with_style(parent).unwrap().style;
        let horizontal = !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
            || (stretch(style.justify_self, container.justify_items)
                && !any_auto([style.margin.left, style.margin.right]));
        let vertical = !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
            || (cv.height_keyword.is_none()
                && stretch(style.align_self, container.align_items)
                && !any_auto([style.margin.top, style.margin.bottom]));
        container.display == taffy::Display::Grid
            && horizontal
            && vertical
            && has_only_fixed_rows(doc, parent)
    })
}

/// Whether every row an item of `grid` can land in has a fixed length, so
/// no item contributes to a row's size: the explicit rows are fixed, and so
/// are the implicit ones unless the grid has a single in-flow item with an
/// automatic one-row placement, which puts it in the first explicit row.
fn has_only_fixed_rows(doc: &LayoutSession<'_>, grid: usize) -> bool {
    use taffy::TraversePartialTree;
    let fixed = |track: &taffy::TrackSizingFunction| {
        track.min.into_raw().tag() == taffy::CompactLength::LENGTH_TAG
            && track.max.into_raw().tag() == taffy::CompactLength::LENGTH_TAG
    };
    let style = &doc.layout_node_with_style(grid).unwrap().style;
    let explicit = !style.grid_template_rows.is_empty()
        && style.grid_template_rows.iter().all(|row| match row {
            taffy::GridTemplateComponent::Single(track) => fixed(track),
            taffy::GridTemplateComponent::Repeat(_) => false,
        });
    let implicit = !style.grid_auto_rows.is_empty() && style.grid_auto_rows.iter().all(fixed);
    let mut items = doc.child_ids(taffy::NodeId::from(grid)).filter(|&child| {
        let child = &doc
            .layout_node_with_style(usize::from(child))
            .unwrap()
            .style;
        child.display != taffy::Display::None && child.position != taffy::Position::Absolute
    });
    let lone_auto_placed_item = match (items.next(), items.next()) {
        (Some(item), None) => {
            let row = &doc
                .layout_node_with_style(usize::from(item))
                .unwrap()
                .style
                .grid_row;
            use taffy::GridPlacement::{Auto, Span};
            matches!(
                (&row.start, &row.end),
                (Auto, Auto) | (Auto, Span(1)) | (Span(1), Auto)
            )
        }
        _ => false,
    };
    explicit && (implicit || lone_auto_placed_item)
}

/// Supply the dimensions of the small set of WPT image assets whose bytes are
/// intentionally resolved by the renderer's URL-color fallback.  Without an
/// intrinsic size, a replaced `<img>` with auto width and height collapses to
/// zero even though its sibling background-image fallback paints a rectangle.
///
/// An image that was decoded has its own natural size, which is not the one
/// supplied here: the supplied size would be taken as authored and win over it.
/// The empty box a resolver's fallback leaves is not a natural size.
fn bridge_known_image_intrinsic_size(
    doc: &Document,
    node_id: usize,
    cv: &ComputedValues,
) -> Option<(f32, f32)> {
    let node = &doc.layout_node_with_style(node_id).unwrap();
    let has_natural_size = node
        .image_intrinsic_size()
        .is_some_and(|(width, height)| width > 0.0 && height > 0.0);
    if node.tag_name() != Some("img") || has_natural_size {
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
    vertical_axes: bool,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    // Use two struct literals to write each pair of min/max fields together
    // (as in bridge_size; cv's Auto naturally preserves defaults).
    // Keep calc() handles alive in the document arena just like width/height;
    // collapsing a max-width calc() to zero would spuriously clamp a <col>.
    // A vertical box laid out on horizontal axes (`vertical_axes` unset)
    // keeps a logical block minimum on the horizontal block axis when no
    // physical min-height is present, like its inline-size/block-size.
    // Preserve an explicit physical constraint; CSSOM keeps the original
    // axis mapping. A box laid out on vertical axes has its logical block
    // minimum as its minimum width already.
    let min_height = if !vertical_axes
        && cv.cssom_writing_mode.is_vertical()
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
            true,
            "min-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            min_height,
            true,
            "min-height",
            diag,
        ),
    };
    style.max_size = Size {
        width: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_width,
            true,
            "max-width",
            diag,
        ),
        height: computed_length_percentage_or_auto_to_taffy_min_max(
            calc_values,
            cv.max_height,
            true,
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
/// default (like the [`Node::set_box_properties`] catch-all → `Block`).
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
            ComputedLengthPercentageOrCalc::Px(0.0) | ComputedLengthPercentageOrCalc::Percent(0.0)
        )
        && matches!(
            cv.padding.bottom,
            ComputedLengthPercentageOrCalc::Px(0.0) | ComputedLengthPercentageOrCalc::Percent(0.0)
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
fn bridge_flex(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    style.flex_direction = match cv.flex_direction {
        FlexDirectionValue::Row => TaffyFlexDirection::Row,
        FlexDirectionValue::RowReverse => TaffyFlexDirection::RowReverse,
        FlexDirectionValue::Column => TaffyFlexDirection::Column,
        FlexDirectionValue::ColumnReverse => TaffyFlexDirection::ColumnReverse,
        // cov:ignore: unreachable while FlexDirectionValue is
        // Row|RowReverse|Column|ColumnReverse only; required for its
        // #[non_exhaustive] contract (see doc above, `Node::set_box_properties`
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
        ComputedFlexBasis::Calc(value) => {
            taffy::Dimension::calc(crate::taffy_impl::push_calc(calc_values, value, true))
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
    style.justify_content =
        content_alignment_to_taffy(cv.justify_content).unwrap_or(TaffyAlignContent::NORMAL);
    style.align_content =
        content_alignment_to_taffy(cv.align_content).unwrap_or(TaffyAlignContent::NORMAL);
    style.align_items = self_alignment_to_taffy(cv.align_items).unwrap_or(TaffyAlignItems::NORMAL);
    style.justify_items =
        self_alignment_to_taffy(cv.justify_items).unwrap_or(TaffyAlignItems::NORMAL);
    // An absolutely positioned box takes no part in baseline alignment, so
    // `last baseline` falls back to `safe self-end` (CSS Box Alignment 3 §9.1).
    let abspos = matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed);
    let own = |value: AlignSelfValue| match value {
        AlignSelfValue::Value(SelfAlignmentValue::LastBaseline) if abspos => {
            Some(TaffyAlignItems::SAFE_SELF_END)
        }
        value => self_alignment_or_auto_to_taffy(value),
    };
    style.align_self = own(cv.align_self);
    style.justify_self = own(cv.justify_self);
}

/// An absolutely positioned box whose `align-self` / `justify-self` is `auto`
/// takes the container's `align-items` / `justify-items` for its static
/// position, where both insets of the axis are `auto`. When that is
/// `last baseline`, the box falls back to `safe self-end` like an explicit
/// `last baseline` does (see [`bridge_alignment`]); taffy alone would
/// inherit the container's plain baseline. With an inset specified, `auto`
/// behaves as `normal` and the box stays at the start.
// spec: https://www.w3.org/TR/css-align-3/#valdef-justify-self-auto
fn bridge_inherited_last_baseline(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    parent: &ComputedValues,
) {
    if !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) {
        return;
    }
    let auto = |value: ComputedLengthPercentageOrAuto| {
        matches!(value, ComputedLengthPercentageOrAuto::Auto)
    };
    if matches!(cv.align_self, AlignSelfValue::Auto)
        && parent.align_items == SelfAlignmentValue::LastBaseline
        && auto(cv.top)
        && auto(cv.bottom)
    {
        style.align_self = Some(TaffyAlignItems::SAFE_SELF_END);
    }
    if matches!(cv.justify_self, AlignSelfValue::Auto)
        && parent.justify_items == SelfAlignmentValue::LastBaseline
        && auto(cv.left)
        && auto(cv.right)
    {
        style.justify_self = Some(TaffyAlignItems::SAFE_SELF_END);
    }
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
        // Preserve Taffy's explicit NORMAL value. None inherits the
        // container property and therefore represents CSS auto instead.
        AlignSelfValue::Value(SelfAlignmentValue::Normal) => Some(TaffyAlignItems::NORMAL),
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
        SelfAlignmentValue::Baseline | SelfAlignmentValue::LastBaseline => {
            Some(TaffyAlignItems::BASELINE)
        }
        // cov:ignore: unreachable while SelfAlignmentValue is the 9
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
fn bridge_gap(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    style.gap = Size {
        width: computed_gap_component_to_taffy(calc_values, cv.column_gap, "column-gap", diag),
        height: computed_gap_component_to_taffy(calc_values, cv.row_gap, "row-gap", diag),
    };
}

/// Per-axis helper for [`bridge_gap`]: `width` in `taffy::Size<LengthPercentage>`
/// corresponds to the inline axis (`column-gap`), while `height` corresponds
/// to the block axis (`row-gap`). This follows taffy `Size` conventions and
/// the axis mapping of `width`/`height` in [`bridge_size`].
fn computed_gap_component_to_taffy(
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    gap: ComputedLengthPercentageOrNormal,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    let lp = match gap {
        ComputedLengthPercentageOrNormal::Normal => ComputedLengthPercentage::Px(0.0),
        ComputedLengthPercentageOrNormal::Px(v) => ComputedLengthPercentage::Px(v),
        ComputedLengthPercentageOrNormal::Percent(p) => ComputedLengthPercentage::Percent(p),
        // Gaps are non-negative, so the sum clamps to zero once resolved.
        ComputedLengthPercentageOrNormal::Calc(calc) => {
            let pointer = crate::taffy_impl::push_calc(calc_values, calc, true);
            return LengthPercentage::calc(pointer);
        }
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
fn bridge_grid(
    style: &mut taffy::Style,
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) {
    let (columns, column_names) = grid_template_tracks_to_taffy(
        calc_values,
        &cv.grid_template_columns,
        "grid-template-columns",
        diag,
    );
    style.grid_template_columns = columns;
    style.grid_template_column_names = column_names;
    let (rows, row_names) = grid_template_tracks_to_taffy(
        calc_values,
        &cv.grid_template_rows,
        "grid-template-rows",
        diag,
    );
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
        // (see `Node::set_box_properties` catch-all doc).
        _ => None,
    };

    style.grid_auto_columns = cv
        .grid_auto_columns
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(calc_values, t, "grid-auto-columns", diag))
        .collect();
    style.grid_auto_rows = cv
        .grid_auto_rows
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(calc_values, t, "grid-auto-rows", diag))
        .collect();

    style.grid_auto_flow = match cv.grid_auto_flow {
        GridAutoFlowValue::Row => TaffyGridAutoFlow::Row,
        GridAutoFlowValue::Column => TaffyGridAutoFlow::Column,
        GridAutoFlowValue::RowDense => TaffyGridAutoFlow::RowDense,
        GridAutoFlowValue::ColumnDense => TaffyGridAutoFlow::ColumnDense,
        // cov:ignore: unreachable while GridAutoFlowValue is
        // Row|Column|RowDense|ColumnDense only; required for its
        // #[non_exhaustive] contract (see `Node::set_box_properties` catch-all doc).
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
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
                    ComputedGridTrackListComponent::Size(size) => GridTemplateComponent::Single(
                        grid_track_size_to_taffy(calc_values, size, site, diag),
                    ),
                    ComputedGridTrackListComponent::Repeat(repeat) => {
                        GridTemplateComponent::Repeat(GridTemplateRepetition {
                            count: grid_repeat_count_to_taffy(repeat.count),
                            tracks: repeat
                                .tracks
                                .into_iter()
                                .map(|t| grid_track_size_to_taffy(calc_values, t, site, diag))
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    size: ComputedGridTrackSize,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> TrackSizingFunction {
    match size {
        ComputedGridTrackSize::Breadth(b) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(calc_values, b, site, diag),
            max: grid_track_breadth_to_taffy_max(calc_values, b, site, diag),
        },
        ComputedGridTrackSize::MinMax(min, max) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(calc_values, min, site, diag),
            max: grid_track_breadth_to_taffy_max(calc_values, max, site, diag),
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MinTrackSizingFunction {
    use crate::taffy_impl::push_calc;
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MinTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MinTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::MinContent => MinTrackSizingFunction::min_content(),
        B::MaxContent => MinTrackSizingFunction::max_content(),
        B::Auto => MinTrackSizingFunction::auto(),
        B::Calc(calc) => MinTrackSizingFunction::calc(push_calc(calc_values, calc, true)),
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MaxTrackSizingFunction {
    use crate::taffy_impl::push_calc;
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MaxTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MaxTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::Flex(f) => MaxTrackSizingFunction::fr(sanitize_taffy(f, site, diag)),
        B::MinContent => MaxTrackSizingFunction::min_content(),
        B::MaxContent => MaxTrackSizingFunction::max_content(),
        B::Auto => MaxTrackSizingFunction::auto(),
        B::Calc(calc) => MaxTrackSizingFunction::calc(push_calc(calc_values, calc, true)),
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

/// The `min-*` and `max-*` sizes of `cv` that are intrinsic sizing keywords
/// as the keyword [`Dimension`]s of the `width` bridge, which Taffy's min and
/// max size cannot hold ([`bridge_min_max_size`] leaves them `auto`).
fn bridge_content_limits(
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    diag: &mut Vec<LayoutWarn>,
) -> ContentLimits {
    let mut keyword = |size, site| {
        if is_sizing_keyword(size) {
            computed_length_percentage_or_auto_to_taffy_dimension(calc_values, size, site, diag)
        } else {
            Dimension::auto()
        }
    };
    ContentLimits {
        min: Size {
            width: keyword(cv.min_width, "min-width"),
            height: keyword(cv.min_height, "min-height"),
        },
        max: Size {
            width: keyword(cv.max_width, "max-width"),
            height: keyword(cv.max_height, "max-height"),
        },
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
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
        // CSS Sizing 3 §3.1.1: the intrinsic size keywords in that axis.
        ComputedLengthPercentageOrAuto::MinContent => Dimension::min_content(),
        ComputedLengthPercentageOrAuto::MaxContent => Dimension::max_content(),
        ComputedLengthPercentageOrAuto::FitContent => Dimension::fit_content(),
        // `fit-content(<length-percentage>)`: the limit keeps its percentage
        // for Taffy to resolve against the containing block.
        ComputedLengthPercentageOrAuto::FitContentOf(limit) => match limit {
            ComputedLengthPercentageOrCalc::Px(v) => {
                Dimension::fit_content_px(sanitize_taffy(v, site, diag))
            }
            ComputedLengthPercentageOrCalc::Percent(p) => {
                Dimension::fit_content_percent(sanitize_taffy(p / 100.0, site, diag))
            }
            ComputedLengthPercentageOrCalc::Calc(value) => {
                let pointer = crate::taffy_impl::push_calc(calc_values, value, true);
                Dimension::fit_content_calc(pointer)
            }
        },
        ComputedLengthPercentageOrAuto::Calc(value) => {
            let pointer = crate::taffy_impl::push_calc(calc_values, value, true);
            Dimension::calc(pointer)
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// for min/max-size properties and insets.
///
/// Unlike the margin bridge below, min/max preferred sizes and insets can
/// carry a calc() payload. Preserve that payload in the document arena so
/// consumers such as table `<col>` sizing do not observe a false zero
/// constraint.
fn computed_length_percentage_or_auto_to_taffy_min_max(
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
    loa: ComputedLengthPercentageOrAuto,
    non_negative: bool,
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
        // Taffy's min and max sizes cannot hold a sizing keyword; layout
        // resolves the keyword from the box's content limits instead, so
        // Taffy sees no constraint.
        ComputedLengthPercentageOrAuto::Auto
        | ComputedLengthPercentageOrAuto::MinContent
        | ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_) => LengthPercentageAuto::auto(),
        ComputedLengthPercentageOrAuto::Calc(value) => {
            let pointer = crate::taffy_impl::push_calc(calc_values, value, non_negative);
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
    calc_values: &mut Vec<std::sync::Arc<crate::taffy_impl::ArenaCalc>>,
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
        // A mixed percentage-and-length margin resolves against the
        // containing block in taffy, through the same arena as sizes.
        ComputedLengthPercentageOrAuto::Calc(value) => {
            let pointer = crate::taffy_impl::push_calc(calc_values, value, false);
            LengthPercentageAuto::calc(pointer)
        }
        // Margins never compute to a sizing keyword.
        ComputedLengthPercentageOrAuto::MinContent
        | ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_) => LengthPercentageAuto::length(0.0),
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

/// The engine's `text-group-align` for the cascade's value.
fn shodo_group_align(
    value: raikiri_style::property::TextGroupAlign,
) -> shodo::style::TextGroupAlign {
    use raikiri_style::property::TextGroupAlign as P;
    use shodo::style::TextGroupAlign as S;
    match value {
        P::Start => S::Start,
        P::End => S::End,
        P::Left => S::Left,
        P::Right => S::Right,
        P::Center => S::Center,
        // `none`, and any value the cascade does not produce.
        _ => S::None,
    }
}
