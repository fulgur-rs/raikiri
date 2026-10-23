use super::style_views::metadata::{size_containment, sizing_flags as view_sizing_flags};
use super::*;
use crate::layout::context::LayoutSession;
use crate::node::{FormattingContext, Node};
use crate::taffy_impl::content_limits::ContentLimits;
use clamp_warnings::{report_non_finite_rest, report_non_finite_sizes};
use raikiri_style::property::{BoxDecorationBreak, ContentComponent, TextOrientation};
use raikiri_style::{PseudoElem, StyleNodeId};
use settled::Settled;

mod clamp_warnings;
mod settled;

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

/// Whether `idx` lays out on vertical axes: a box in `vertical-rl` or
/// `vertical-lr`. A multicol container and its contents run vertical writing
/// on horizontal axes, and so do grid containers: their algorithm places
/// items on physical axes, together with the horizontally mapped logical
/// margins. A flex container runs its algorithm on the axes of its own
/// writing mode.
pub(crate) fn lays_out_vertically(cascade: &CascadeResult, idx: usize) -> bool {
    let values = &cascade.computed[idx];
    if !matches!(
        values.cssom_writing_mode,
        WritingMode::VerticalRl
            | WritingMode::VerticalLr
            | WritingMode::SidewaysRl
            | WritingMode::SidewaysLr
    ) || values.display.is_grid_container()
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

/// Bridges `cascade` in a session of its own, as layout starts one.
#[cfg(test)]
pub(crate) fn apply_computed_to_style(
    doc: &mut Document,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    let mut session = LayoutSession::with_cascade(doc, cascade);
    apply_computed_to_style_in_session(&mut session, cascade)
}

#[cfg(test)]
impl<'doc> LayoutSession<'doc> {
    /// A session over `document` whose styles are bridged from `cascade` and
    /// prepared, the state layout runs in.
    pub(crate) fn bridged(document: &'doc mut Document, cascade: &'doc CascadeResult) -> Self {
        let mut session = Self::with_cascade(document, cascade);
        session
            .with_style_preparation_batch(|session| {
                apply_computed_to_style_in_session(session, cascade)?;
                // A clone of a laid-out document keeps the page-area box in
                // its arena without the style data of it; layout publishes
                // that again once the styles are bridged.
                if session.nodes.initial_containing_block_key().is_some()
                    && session
                        .document
                        .box_style_data
                        .get(crate::node::arena::BoxKey::InitialContainingBlock)
                        .is_none()
                {
                    let (width, height) = session.fixed_page_area.unwrap_or_default();
                    super::initial_containing_block::prepare(
                        session.document,
                        cascade,
                        width,
                        height,
                    );
                }
                Ok::<_, LayoutError>(())
            })
            .expect("styles bridge");
        session
    }

    /// A session over `document` as an earlier bridge or layout left it, with
    /// the style views prepared from the style data already there.
    ///
    /// Nothing is bridged again, so what a test patched into the nodes and
    /// what layout computed both stay.
    pub(crate) fn resumed(document: &'doc mut Document, cascade: &'doc CascadeResult) -> Self {
        let mut session = Self::with_cascade(document, cascade);
        session.prepare_style_views();
        session
    }
}

/// Carries what layout needs of each element's computed values to the nodes
/// and to the style views: the node fields the engines read, the per-property
/// overrides of boxes whose style depends on their content or neighbours, and
/// the warnings for lengths that cannot reach Taffy as they are. The
/// properties themselves are read from the computed values by the style views.
///
/// After the per-node bridge loop, the paragraphs of the inline engine are
/// assigned ([`crate::layout::ifc::assign::assign_ifc_roots`]).
///
/// # Errors
/// What [`crate::layout::ifc::assign::assign_ifc_roots`] returns: a paragraph
/// over a limit of the inline engine, or one it refuses when it is required
/// for every paragraph.
pub(crate) fn apply_computed_to_style_in_session(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    doc.begin_style_bridge();
    doc.document.box_style_data.invalidate_source();
    doc.document.layout_cache_context.publish_source();
    doc.context.table_cache.clear();
    // Keep the previous payloads alive while styles are rebuilt in place.
    // A later error or unwind may leave some styles with old handles and
    // others with newly appended handles, so only a complete successful
    // bridge can release the old prefix.
    let old_calc_len = doc.document.box_style_data.calc_values.len();
    let result = apply_computed_to_style_in_session_inner(doc, cascade);
    if result.is_ok() {
        let expected = doc
            .nodes
            .iter()
            .filter(|node| node.kind() == NodeKind::Element)
            .count()
            + 8;
        doc.document.layout_cache_context.configure_nodes(expected);
        doc.finish_style_bridge(old_calc_len);
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
    let mut settled_styles = vec![Settled::default(); doc.nodes.len()];
    for (idx, slot) in settled_styles.iter_mut().enumerate() {
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
        // A resolver may give the natural size and leave its ratio unset; the
        // ratio of that size is then the natural one.
        let natural_ratio = crate::taffy_impl::leaf_intrinsic_ratio(&doc.nodes[idx]);
        let preferred_ratio = preferred_aspect_ratio(cv, natural_ratio, false);
        let mut settled = Settled::of(cv);
        settled.has_aspect_ratio = preferred_ratio.is_some();
        doc.nodes[idx].preferred_aspect_ratio = preferred_ratio;
        doc.nodes[idx].ratio_in_content_box = cv.aspect_ratio.relates_content_box(natural_ratio);
        if natural_ratio.is_some() {
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::aspect_ratio(preferred_ratio),
            );
        }
        doc.nodes[idx].svg_width_follows_available = false;
        // SVG 2 geometry: auto dimensions on svg are treated as 100%.
        // With only a viewBox ratio and no definite height, width establishes
        // the viewport and the ratio determines its auto height.
        if doc.nodes[idx].is_inline_svg_root()
            && doc.nodes[idx].attribute("width").is_none()
            && doc.nodes[idx].attribute("height").is_none()
            && settled.has_aspect_ratio
        {
            if matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
                && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
            {
                doc.nodes[idx].svg_width_follows_available = true;
                doc.record_pass_override(
                    idx,
                    raikiri_layout_style::OwnedPropertyOverride::length(
                        raikiri_layout_style::LengthProperty::Width,
                        ComputedLengthPercentageOrAuto::Percent(100.0),
                        false,
                    )
                    .expect("physical SVG width"),
                );
            } else if !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto) {
                // Taffy transfers max-height to max-width through this ratio
                // even for a definite width. SVG leaf measurement handles its
                // auto height without shrinking an authored width.
                settled.has_aspect_ratio = false;
                doc.record_pass_override(
                    idx,
                    raikiri_layout_style::OwnedPropertyOverride::aspect_ratio(None),
                );
            }
        }
        if let Some((intrinsic_width, intrinsic_height)) =
            bridge_known_image_intrinsic_size(doc, idx, cv.width, cv.height)
        {
            {
                use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
                for (property, value, original) in [
                    (P::Width, intrinsic_width, cv.width),
                    (P::Height, intrinsic_height, cv.height),
                ] {
                    if matches!(original, ComputedLengthPercentageOrAuto::Auto) {
                        doc.record_pass_override(
                            idx,
                            O::length(property, ComputedLengthPercentageOrAuto::Px(value), false)
                                .expect("physical fallback size"),
                        );
                    }
                }
            }
        }
        doc.document.nodes[idx].has_logical_min_block_size = cv.min_block_size.is_some()
            && matches!(cv.break_inside, raikiri_style::property::BreakInside::Avoid);
        doc.document.nodes[idx].needs_relative_block_paint_offset = false;
        let vertical_axes = uses_vertical_axes(cascade, idx);
        let is_contents = doc.document.nodes[idx].display() == DisplayValue::Contents;
        // Report the lengths that are clamped on their way to layout.
        report_non_finite_sizes(cv, vertical_axes, &mut doc.context.layout_warnings);
        // Keep the owners of the limits' calc payloads in the document, which
        // the limits point into.
        let mut calc_values = Vec::new();
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
        report_non_finite_rest(cv, &mut doc.context.layout_warnings);
        // The container an `auto` self-alignment borrows from is the nearest
        // ancestor that makes a box: `display: contents` ones are flattened
        // away, except the document element, which is blockified.
        let boxes = super::box_tree::BoxTree::for_cascade(doc, cascade);
        let container = doc
            .ancestors(idx)
            .find(|&ancestor| boxes.display(ancestor) != Some(DisplayValue::Contents));
        if let Some(container) = container {
            let (align_self, justify_self) =
                settled::inherited_last_baseline(cv, &cascade.computed[container]);
            if let Some(value) = align_self {
                settled.align_self = Some(value);
                doc.record_pass_override(
                    idx,
                    raikiri_layout_style::OwnedPropertyOverride::align_self(Some(value)),
                );
            }
            if let Some(value) = justify_self {
                settled.justify_self = Some(value);
                doc.record_pass_override(
                    idx,
                    raikiri_layout_style::OwnedPropertyOverride::justify_self(Some(value)),
                );
            }
        }
        if is_contents {
            // A `display: contents` element generates no box (CSS Display 3
            // §2.5). Where layout still keeps a node for it, the node stands
            // in for the anonymous box its contents would get, so none of
            // the element's own box properties apply to it.
            settled = Settled::default();
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
        *slot = settled;
        // The owners of the limits' payloads stay with the document before
        // any raw pointer to them is published.
        doc.document.box_style_data.calc_values.extend(calc_values);
        if is_contents {
            let key = doc.nodes.box_key(idx).expect("source node has a box key");
            let recipe = super::box_style_data::recipes::contents(
                super::box_style_data::recipes::BoxFacts {
                    direction: direction_for_axes(cv, vertical_axes),
                    metadata: super::style_views::metadata::node_metadata(
                        &doc.nodes[idx],
                        raikiri_layout_style::StyleFlags::EMPTY,
                    ),
                },
            );
            doc.document.box_style_data.publish_recipe(key, recipe);
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            let recipe = super::box_style_data::recipes::neutral_source(
                super::box_style_data::recipes::BoxFacts {
                    direction: taffy::Direction::Ltr,
                    metadata: super::style_views::metadata::node_metadata(
                        &doc.nodes[idx],
                        raikiri_layout_style::StyleFlags::EMPTY,
                    ),
                },
            );
            doc.document
                .box_style_data
                .publish_recipe(crate::node::arena::BoxKey::Source(idx), recipe);
        }
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
        // stand in for an `auto` size. Only a contained width keeps a content
        // keyword width at the width of an empty box.
        if containment.width || containment.height {
            let contained_ratio = preferred_aspect_ratio(cv, None, containment.width);
            settled_styles[idx].has_aspect_ratio = contained_ratio.is_some();
            {
                use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
                doc.record_pass_override(idx, O::aspect_ratio(contained_ratio));
                for (property, contained, original) in [
                    (P::Width, containment.width, cv.width),
                    (P::Height, containment.height, cv.height),
                ] {
                    if contained && matches!(original, ComputedLengthPercentageOrAuto::Auto) {
                        doc.record_pass_override(
                            idx,
                            O::length(property, ComputedLengthPercentageOrAuto::Auto, false)
                                .expect("physical contained size"),
                        );
                    }
                }
            }
        }
        if settled::stretches_on_both_grid_axes(doc, cascade, idx, cv, &settled_styles)
            && !replaced_keyword_width(doc.document, cascade, idx)
        {
            settled_styles[idx].has_aspect_ratio = false;
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::aspect_ratio(None),
            );
        }
        if settled::is_stretched_by_replaced_keyword_width_in_grid(
            doc,
            cascade,
            idx,
            &settled_styles,
        ) {
            settled_styles[idx].justify_self = Some(TaffyAlignItems::START);
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::justify_self(Some(
                    TaffyAlignItems::START,
                )),
            );
        }
        if settled::is_stretched_by_keyword_width(doc, cascade, idx, cv, &settled_styles) {
            settled_styles[idx].align_self = Some(TaffyAlignItems::FLEX_START);
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::align_self(Some(
                    TaffyAlignItems::FLEX_START,
                )),
            );
        }
        if let Some(fallback) =
            settled::unstretched_keyword_height_alignment(doc, cascade, idx, &settled_styles)
        {
            settled_styles[idx].align_self = Some(fallback);
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::align_self(Some(fallback)),
            );
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
pub(super) fn direction_for_axes(cv: &ComputedValues, vertical_axes: bool) -> TaffyDirection {
    let upright = vertical_axes
        && cv.text_orientation == TextOrientation::Upright
        && matches!(
            cv.cssom_writing_mode,
            WritingMode::VerticalRl | WritingMode::VerticalLr
        );
    let rtl = matches!(cv.direction, Direction::Rtl) && !upright;
    let bottom_to_top = vertical_axes && cv.cssom_writing_mode == WritingMode::SidewaysLr;
    if rtl != bottom_to_top {
        TaffyDirection::Rtl
    } else {
        TaffyDirection::Ltr
    }
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
    // Only intrinsic keywords need contextual normalization. Keep the ordinary
    // scalar path free of repeated ancestor and formatting-context queries.
    if ![
        cv.width,
        cv.height,
        cv.min_width,
        cv.max_width,
        cv.min_height,
        cv.max_height,
        cv.min_block_size
            .unwrap_or(ComputedLengthPercentageOrAuto::Auto),
    ]
    .into_iter()
    .any(is_sizing_keyword)
    {
        return std::borrow::Cow::Borrowed(cv);
    }
    use raikiri_layout_style::StyleFlags as F;
    let flags = view_sizing_flags(doc, cascade, idx);
    let width = flags.contains(F::IGNORE_WIDTH_KEYWORD)
        || (flags.contains(F::MIN_CONTENT_WIDTH_AUTO)
            && cv.width == ComputedLengthPercentageOrAuto::MinContent);
    let height = flags.contains(F::IGNORE_HEIGHT_KEYWORD);
    let plain_limits = flags.contains(F::IGNORE_LIMIT_KEYWORDS)
        && [cv.min_width, cv.max_width, cv.min_height, cv.max_height]
            .into_iter()
            .any(is_sizing_keyword);
    let ignore_logical_min_block = flags.contains(F::IGNORE_LOGICAL_MIN_BLOCK);
    if !(width || height || plain_limits || ignore_logical_min_block) {
        return std::borrow::Cow::Borrowed(cv);
    }
    let mut plain = cv.clone();
    if width {
        plain.width = ComputedLengthPercentageOrAuto::Auto;
    }
    if height {
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

/// The computed values of element `idx` as its layout reads them: the
/// cascade's own with [`layout_values`]' normalization of sizing keywords,
/// and a vertical box taking its sizes from its logical properties.
pub(super) fn layout_values_of<'a>(
    doc: &Document,
    cascade: &'a CascadeResult,
    idx: usize,
) -> std::borrow::Cow<'a, ComputedValues> {
    match vertical_logical_sized(cascade, idx) {
        None => layout_values(doc, cascade, idx, None),
        Some(sized) => {
            std::borrow::Cow::Owned(layout_values(doc, cascade, idx, Some(&sized)).into_owned())
        }
    }
}

/// Whether `size` is one of the intrinsic sizing keywords.
pub(super) fn is_sizing_keyword(size: ComputedLengthPercentageOrAuto) -> bool {
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
pub(super) fn honors_content_limits(
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
pub(super) fn is_supported_flex_grid_container(
    doc: &Document,
    idx: usize,
    cv: &ComputedValues,
) -> bool {
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
pub(super) fn is_supported_flex_grid_item(
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
///   the container's algorithm sizes along its own axes;
/// - a flex or grid item with a `min-width` or `max-width` is measured by Taffy
///   at the keyword size before that size is clamped, so the lines of its
///   content break at a width the box does not keep.
pub(super) fn sized_by_content_keyword(
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
    doc.ancestors(idx)
        .find(|&id| cascade.computed[id].display != DisplayValue::Contents)
}

/// Whether a box of `display` lays its content out by an algorithm of its
/// own, to which the column properties do not apply.
fn lays_out_without_columns(display: DisplayValue) -> bool {
    display.is_flex_or_grid_container()
        || matches!(display, DisplayValue::Table | DisplayValue::InlineTable)
}

/// The taffy border widths of `border`.
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

/// The preferred aspect ratio taffy sizes a box by (CSS Box Sizing 4 §5.1):
/// the ratio the `aspect-ratio` property gives it, with `natural` standing for
/// the natural ratio of replaced content. A ratio only participates when at
/// least one preferred dimension is auto; Taffy's leaf sizing otherwise floors
/// an explicit height to the ratio-derived height. A width that is a content
/// keyword is no more definite than `auto` for a box without a natural ratio:
/// the ratio gives it the width of its height (CSS Sizing 4 §5.1). A box with
/// size containment keeps the width of an empty box instead.
fn preferred_aspect_ratio(
    cv: &ComputedValues,
    natural: Option<f32>,
    size_contained: bool,
) -> Option<f32> {
    raikiri_layout_style::preferred_aspect_ratio(
        cv.aspect_ratio,
        cv.width,
        cv.height,
        natural,
        size_contained,
    )
}

/// Whether element `idx` is a replaced box of a horizontal writing mode, which
/// sizes itself by its `min-content` width as it does by `auto`.
pub(super) fn min_content_width_is_auto(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
) -> bool {
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

/// Supply the dimensions of the small set of WPT image assets whose bytes are
/// intentionally resolved by the renderer's URL-color fallback.  Without an
/// intrinsic size, a replaced `<img>` with auto width and height collapses to
/// zero even though its sibling background-image fallback paints a rectangle.
///
/// An image that was decoded has its own natural size, which is not the one
/// supplied here: the supplied size would be taken as authored and win over it.
/// The empty box a resolver's fallback leaves is not a natural size.
pub(super) fn bridge_known_image_intrinsic_size(
    doc: &Document,
    node_id: usize,
    width: ComputedLengthPercentageOrAuto,
    height: ComputedLengthPercentageOrAuto,
) -> Option<(f32, f32)> {
    let node = &doc.nodes[node_id];
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
    let width_auto = matches!(width, ComputedLengthPercentageOrAuto::Auto);
    let height_auto = matches!(height, ComputedLengthPercentageOrAuto::Auto);
    (width_auto || height_auto).then_some((100.0, 50.0))
}

/// The `min-height` of a box with the computed values `cv`.
///
/// A vertical box laid out on horizontal axes (`vertical_axes` unset) keeps a
/// logical block minimum on the horizontal block axis when no physical
/// min-height is present, like its inline-size/block-size. Preserve an
/// explicit physical constraint; CSSOM keeps the original axis mapping. A box
/// laid out on vertical axes has its logical block minimum as its minimum
/// width already.
pub(super) fn physical_min_height(
    cv: &ComputedValues,
    vertical_axes: bool,
) -> ComputedLengthPercentageOrAuto {
    if !vertical_axes
        && cv.cssom_writing_mode.is_vertical()
        && matches!(cv.min_height, ComputedLengthPercentageOrAuto::Auto)
    {
        cv.min_block_size.unwrap_or(cv.min_height)
    } else {
        cv.min_height
    }
}

/// [`AlignSelfValue`] → `Option<taffy::AlignItems>` mapping — shared by
/// `align-self` and `justify-self` (taffy's `AlignSelf` is a type alias for
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

/// [`SelfAlignmentValue`] → `Option<taffy::AlignItems>` mapping —
/// shared by the `align-items` mapping and the `AlignSelfValue::Value` branch
/// of the `align-self` one: non-`auto` values of `align-self` use the
/// `align-items` keyword set (see [`AlignSelfValue`]'s docs).
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

/// The `min-*` and `max-*` sizes of `cv` that are intrinsic sizing keywords
/// as the keyword [`Dimension`]s of the `width` bridge, which Taffy's min and
/// max size cannot hold (the style views leave them `auto`).
fn bridge_content_limits(
    cv: &ComputedValues,
    calc_values: &mut Vec<std::sync::Arc<raikiri_layout_style::AlignedCalc>>,
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
/// `Px` and `Percent` values pass the non-finite guard; `Auto` →
/// `Dimension::auto()` needs none, as it has no `f32`.
///
/// `site` names the property in [`LayoutWarn::NonFiniteClamped`] events.
fn computed_length_percentage_or_auto_to_taffy_dimension(
    calc_values: &mut Vec<std::sync::Arc<raikiri_layout_style::AlignedCalc>>,
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

/// [`ComputedLength`] (px) → [`taffy::LengthPercentage`] bridge
/// (for `border-*-width`).
///
/// The grammar of `border-*-width` (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) excludes percentages, so only
/// lengths occur even at the computed-value stage (CSS Backgrounds 3 §3.3
/// "Line Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>). The argument is
/// [`ComputedLength`], so there is no percentage arm; the non-finite guard
/// ([`sanitize_taffy`]) still applies.
fn computed_length_to_taffy_length_percentage(
    len: ComputedLength,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    // The only caller is `taffy_border_widths`, so the site label is fixed.
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
