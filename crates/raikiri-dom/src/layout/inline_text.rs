use super::*;

/// The CSS-collapsible whitespace set (space / tab / LF / FF / CR).
/// `char::is_whitespace` also includes NBSP, which must not collapse, so do
/// not use it here (an approximation of CSS Text 3 §4.1).
fn is_collapsible_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

pub(crate) fn prepare_ch_box_values_before_taffy(doc: &mut Document, cascade: &CascadeResult) {
    // `ch` is measured with the inline engine's fonts, the ones the text is
    // laid out with.
    let Some(fonts) = doc.ifc.as_ref().map(|state| state.fonts.clone()) else {
        return;
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                let used = provenance.factor
                    * crate::layout::ifc::ch::ch_advance(&fonts, &provenance.font);
                if used.is_nan() {
                    0.0
                } else {
                    used.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
                }
            })
        };
        let width = measure(&cv.width_ch).map(|value| value.max(0.0));
        let height = measure(&cv.height_ch).map(|value| value.max(0.0));
        let padding = (
            measure(&cv.padding_ch.top).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.right).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.bottom).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.left).map(|value| value.max(0.0)),
        );
        let margin = (
            measure(&cv.margin_ch.top),
            measure(&cv.margin_ch.right),
            measure(&cv.margin_ch.bottom),
            measure(&cv.margin_ch.left),
        );
        let style = &mut doc.nodes[idx].style;
        if let Some(width) = width {
            style.size.width = Dimension::length(width);
        }
        if let Some(height) = height {
            style.size.height = Dimension::length(height);
        }
        if let Some(top) = padding.0 {
            style.padding.top = LengthPercentage::length(top);
        }
        if let Some(right) = padding.1 {
            style.padding.right = LengthPercentage::length(right);
        }
        if let Some(bottom) = padding.2 {
            style.padding.bottom = LengthPercentage::length(bottom);
        }
        if let Some(left) = padding.3 {
            style.padding.left = LengthPercentage::length(left);
        }
        if let Some(top) = margin.0 {
            style.margin.top = LengthPercentageAuto::length(top);
        }
        if let Some(right) = margin.1 {
            style.margin.right = LengthPercentageAuto::length(right);
        }
        if let Some(bottom) = margin.2 {
            style.margin.bottom = LengthPercentageAuto::length(bottom);
        }
        if let Some(left) = margin.3 {
            style.margin.left = LengthPercentageAuto::length(left);
        }
    }
}

// A helper for the replaced-only post-pass below: mixed inline rows must retain
// Taffy's computed text/replaced baselines rather than being reflowed as image
// and whitespace runs.
fn inline_root_has_unhandled_baseline_content(
    doc: &Document,
    parent_id: usize,
    cascade: &CascadeResult,
) -> bool {
    doc.nodes[parent_id].children.iter().any(|&child| {
        let child_cv = &cascade.computed[child];
        match &doc.nodes[child].data {
            NodeData::Text(text) => {
                let has_text = !text.text_content.is_empty();
                let has_preserved_break = has_text
                    && !matches!(
                        child_cv.white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    )
                    && text
                        .text_content
                        .chars()
                        .any(|ch| matches!(ch, '\n' | '\r' | '\x0C'));
                let has_non_whitespace = has_text
                    && !text
                        .text_content
                        .chars()
                        .all(|ch| is_collapsible_ws(ch) || ch == '\u{00A0}');
                has_preserved_break || has_non_whitespace
            }
            NodeData::Element(_) => doc.nodes[child].tag_name() != Some("img"),
            _ => false,
        }
    })
}

fn relative_position_x_offset(
    cascade: &CascadeResult,
    child_id: usize,
    containing_width: f32,
) -> f32 {
    let child_cv = &cascade.computed[child_id];
    if !matches!(
        child_cv.position,
        PositionValue::Relative | PositionValue::Sticky
    ) {
        return 0.0;
    }
    let left =
        super::page::used_computed_length_percentage_or_auto(child_cv.left, containing_width);
    let right =
        super::page::used_computed_length_percentage_or_auto(child_cv.right, containing_width);
    left.or_else(|| right.map(|value| -value)).unwrap_or(0.0)
}

fn relative_position_y_offset(
    cascade: &CascadeResult,
    child_id: usize,
    containing_height: f32,
) -> f32 {
    let child_cv = &cascade.computed[child_id];
    if !matches!(
        child_cv.position,
        PositionValue::Relative | PositionValue::Sticky
    ) {
        return 0.0;
    }
    let top = super::page::used_computed_length_percentage_or_auto(child_cv.top, containing_height);
    let bottom =
        super::page::used_computed_length_percentage_or_auto(child_cv.bottom, containing_height);
    top.or_else(|| bottom.map(|value| -value)).unwrap_or(0.0)
}

// cov:ignore: exercised by resource-enabled ignored WPT reftests; default coverage has no sparse asset run.
pub(crate) fn realign_inline_replaced_children(doc: &mut Document, cascade: &CascadeResult) {
    for parent_id in 0..doc.nodes.len() {
        if !doc.nodes[parent_id]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
        {
            continue;
        }
        let has_replaced_child = doc.nodes[parent_id].children.iter().any(|&child| {
            doc.nodes[child].is_in_document() && doc.nodes[child].tag_name() == Some("img")
        });
        if !has_replaced_child {
            continue;
        }
        let baseline_aligned =
            doc.nodes[parent_id].style.align_items == Some(TaffyAlignItems::BASELINE);
        if baseline_aligned && inline_root_has_unhandled_baseline_content(doc, parent_id, cascade) {
            // Taffy's positions must survive on mixed rows and rows with
            // explicit breaks that already have line-specific baseline geometry.
            continue;
        }
        let parent_width = doc.nodes[parent_id].unrounded_layout.size.width.max(0.0);
        if !parent_width.is_finite() || parent_width <= 0.0 {
            continue;
        }
        let line_height = line_height_px(&cascade.computed[parent_id]).max(0.0);
        if line_height <= 0.0 || !line_height.is_finite() {
            continue;
        }
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut line_height_used = line_height;
        let mut baseline_for_line = None;
        // Keep NBSP-connected inline content together. U+00A0 is Glue in
        // UAX 14 and forbids breaks before and after, so CSS line breaking
        // must not split an image plus NBSP plus image run across lines.
        // Group maximal runs of images linked by NBSP-containing whitespace
        // and wrap each group atomically. Collapsible-only whitespace stays
        // breakable and keeps the existing drop-at-wrap behavior.
        let children = doc.nodes[parent_id].children.clone();
        const KIND_SKIP: u8 = 0;
        const KIND_IMG: u8 = 1;
        const KIND_GLUE: u8 = 2;
        const KIND_BREAKABLE: u8 = 3;
        const KIND_BR: u8 = 4;
        let mut kinds: Vec<u8> = Vec::with_capacity(children.len());
        let mut widths: Vec<f32> = Vec::with_capacity(children.len());
        let mut heights: Vec<f32> = Vec::with_capacity(children.len());
        let mut collapsibles: Vec<bool> = Vec::with_capacity(children.len());
        for &child in &children {
            if !doc.nodes[child].is_in_document() {
                kinds.push(KIND_SKIP);
                widths.push(0.0);
                heights.push(0.0);
                collapsibles.push(false);
                continue;
            }
            if doc.nodes[child].tag_name() == Some("br") {
                kinds.push(KIND_BR);
                widths.push(0.0);
                heights.push(0.0);
                collapsibles.push(false);
                continue;
            }
            let is_inline_whitespace = matches!(
                &doc.nodes[child].data,
                NodeData::Text(text)
                    if !text.text_content.is_empty()
                        && text
                            .text_content
                            .chars()
                            .all(|ch| is_collapsible_ws(ch) || ch == '\u{00A0}')
            );
            let is_collapsible_whitespace = is_inline_whitespace
                && matches!(
                    cascade.computed[child].white_space,
                    WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine
                )
                && matches!(
                    &doc.nodes[child].data,
                    NodeData::Text(text)
                        if text.text_content.chars().all(is_collapsible_ws)
                );
            if is_inline_whitespace {
                let width =
                    doc.nodes[child].unrounded_layout.size.width.max(
                        style_dimension_length(doc.nodes[child].style.size.width).unwrap_or(0.0),
                    );
                if width <= 0.0 || !width.is_finite() {
                    kinds.push(KIND_SKIP);
                    widths.push(0.0);
                    heights.push(0.0);
                    collapsibles.push(false);
                    continue;
                }
                let has_nbsp = matches!(
                    &doc.nodes[child].data,
                    NodeData::Text(text) if text.text_content.contains('\u{00A0}')
                );
                if has_nbsp {
                    kinds.push(KIND_GLUE);
                } else {
                    kinds.push(KIND_BREAKABLE);
                }
                widths.push(width);
                heights.push(0.0);
                collapsibles.push(is_collapsible_whitespace);
                continue;
            }
            if doc.nodes[child].tag_name() == Some("img") {
                let width = doc.nodes[child].unrounded_layout.size.width.max(0.0);
                let height = doc.nodes[child].unrounded_layout.size.height.max(0.0);
                if width <= 0.0 || !width.is_finite() {
                    kinds.push(KIND_SKIP);
                    widths.push(0.0);
                    heights.push(0.0);
                    collapsibles.push(false);
                    continue;
                }
                kinds.push(KIND_IMG);
                widths.push(width);
                heights.push(height);
                collapsibles.push(false);
                continue;
            }
            kinds.push(KIND_SKIP);
            widths.push(0.0);
            heights.push(0.0);
            collapsibles.push(false);
        }
        // Partition participating positions into wrap-atomic groups. Images
        // join the open group only through an adjacent NBSP glue node, so
        // back-to-back images without NBSP between them stay separable and
        // keep the existing wrapping behavior. Each collapsible-only or
        // preserved-space run without NBSP forms its own single-item group.
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut group_for_pos: Vec<Option<usize>> = vec![None; children.len()];
        let mut current: Vec<usize> = Vec::new();
        for pos in 0..children.len() {
            match kinds[pos] {
                KIND_IMG => {
                    let glued = current.last().is_some_and(|&last| kinds[last] == KIND_GLUE);
                    if current.is_empty() || !glued {
                        if !current.is_empty() {
                            let gid = groups.len();
                            for &p in &current {
                                group_for_pos[p] = Some(gid);
                            }
                            groups.push(std::mem::take(&mut current));
                        }
                        current.push(pos);
                    } else {
                        current.push(pos);
                    }
                }
                KIND_GLUE => {
                    current.push(pos);
                }
                KIND_BREAKABLE => {
                    if !current.is_empty() {
                        let gid = groups.len();
                        for &p in &current {
                            group_for_pos[p] = Some(gid);
                        }
                        groups.push(std::mem::take(&mut current));
                    }
                    let gid = groups.len();
                    groups.push(vec![pos]);
                    group_for_pos[pos] = Some(gid);
                }
                _ => {
                    if !current.is_empty() {
                        let gid = groups.len();
                        for &p in &current {
                            group_for_pos[p] = Some(gid);
                        }
                        groups.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        if !current.is_empty() {
            let gid = groups.len();
            for &p in &current {
                group_for_pos[p] = Some(gid);
            }
            groups.push(current);
        }
        let group_widths: Vec<f32> = groups
            .iter()
            .map(|group| group.iter().map(|&p| widths[p]).sum())
            .collect();
        for pos in 0..children.len() {
            let child = children[pos];
            let kind = kinds[pos];
            if kind == KIND_SKIP {
                continue;
            }
            if kind == KIND_BR {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                continue;
            }
            let width = widths[pos];
            let height = heights[pos];
            let is_collapsible_whitespace = collapsibles[pos];
            let gid = group_for_pos[pos].expect("participating position has a group");
            let group = &groups[gid];
            let is_first_in_group = group.first().is_some_and(|&first| first == pos);
            let is_glue_group = group.len() > 1;
            if is_glue_group {
                // A multi-item group holds at least one NBSP glue node by
                // construction, so the whole run moves as one unit. Only the
                // first item tests the group width. Later items skip the wrap
                // test so the run can overflow the line rather than split.
                if is_first_in_group {
                    let group_width = group_widths[gid];
                    if x > 0.0 && x + group_width > parent_width + 0.01 {
                        x = 0.0;
                        y += line_height_used;
                        line_height_used = line_height;
                        baseline_for_line = None;
                    }
                }
            } else {
                if x > 0.0 && x + width > parent_width + 0.01 {
                    x = 0.0;
                    y += line_height_used;
                    line_height_used = line_height;
                    baseline_for_line = None;
                    if is_collapsible_whitespace {
                        continue;
                    }
                }
                if is_collapsible_whitespace && x == 0.0 {
                    // CSS-collapsible leading whitespace is removed at a new line.
                    // U+00A0 is not collapsible and keeps its measured advance.
                    continue;
                }
            }
            let taffy_y = doc.nodes[child].unrounded_layout.location.y;
            let relative_offset_y = relative_position_y_offset(
                cascade,
                child,
                doc.nodes[parent_id].unrounded_layout.size.height.max(0.0),
            );
            let flow_taffy_y = taffy_y - relative_offset_y;
            let child_y = if !baseline_aligned {
                y + relative_offset_y
            } else if doc.nodes[child].style.align_self == Some(TaffyAlignItems::FLEX_START) {
                flow_taffy_y + y + relative_offset_y
            } else if doc.nodes[child].style.align_self == Some(TaffyAlignItems::FLEX_END) {
                if y == 0.0 {
                    flow_taffy_y + relative_offset_y
                } else {
                    y + line_height_used - height + relative_offset_y
                }
            } else {
                let baseline_offset = *baseline_for_line.get_or_insert(if y == 0.0 {
                    flow_taffy_y + height
                } else {
                    height
                });
                y + baseline_offset - height + relative_offset_y
            };
            let relative_offset_x = relative_position_x_offset(
                cascade,
                child,
                doc.nodes[parent_id].unrounded_layout.size.width.max(0.0),
            );
            doc.nodes[child].unrounded_layout.location.x = x + relative_offset_x;
            doc.nodes[child].unrounded_layout.location.y = child_y;
            x += width;
            line_height_used = line_height_used.max(height);
        }
        let used_height = if x > 0.0 || y == 0.0 {
            y + line_height_used
        } else {
            y
        };
        let taffy_height = doc.nodes[parent_id].unrounded_layout.size.height;
        doc.nodes[parent_id].unrounded_layout.size.height = if baseline_aligned {
            taffy_height.max(used_height)
        } else {
            used_height
        };
    }
}

// Text indentation is otherwise applied to TextData layouts, so an empty
// inline block has no text layout to receive the first-line offset. Keep this
// post-layout bridge narrow: only one empty inline-block in a horizontal LTR
// block, with optional collapsible whitespace, no modifiers or forced breaks,
// and normal-flow positioning can be offset without reflowing a line.
pub(crate) fn realign_single_empty_inline_block_indent(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    for parent_id in 0..doc.nodes.len() {
        // The inline engine places the inline-block after the indent itself.
        if doc.nodes[parent_id].is_ifc_root() {
            continue;
        }
        let cv = &cascade.computed[parent_id];
        if !matches!(cv.display, DisplayValue::Block | DisplayValue::InlineBlock) {
            continue;
        }
        if cv.direction != Direction::Ltr
            || cv.writing_mode != WritingMode::HorizontalTb
            || cv.text_indent_hanging
            || cv.text_indent_each_line
            || cv.text_indent_ch_factor.is_some()
            || !matches!(
                cv.text_align,
                TextAlign::Start | TextAlign::Left | TextAlign::Right
            )
        {
            continue;
        }

        let mut inline_block = None;
        let mut unsupported_content = false;
        for &child in &doc.nodes[parent_id].children {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if is_forced_line_break(doc, child, cascade) {
                unsupported_content = true;
                break;
            }
            match &doc.nodes[child].data {
                NodeData::Text(text)
                    if matches!(
                        cascade.computed[child].white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    ) && text.text_content.chars().all(is_css_white_space) => {}
                NodeData::Element(_)
                    if cascade.computed[child].display == DisplayValue::InlineBlock
                        && cascade.computed[child].float == FloatValue::None
                        && matches!(
                            cascade.computed[child].position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        )
                        && doc.nodes[child].children.is_empty() =>
                {
                    if inline_block.replace(child).is_some() {
                        unsupported_content = true;
                        break;
                    }
                }
                _ => {
                    unsupported_content = true;
                    break;
                }
            }
        }
        if unsupported_content {
            continue;
        }
        let Some(inline_block) = inline_block else {
            continue;
        };

        let content_width = doc.nodes[parent_id].unrounded_layout.content_box_width();
        if !content_width.is_finite() || content_width < 0.0 {
            continue;
        }
        let indent = bounded_text_indent_amount(cv.text_indent, content_width, None);
        if !indent.is_finite() || indent == 0.0 {
            continue;
        }
        doc.nodes[inline_block].unrounded_layout.location.x += indent;
    }
}

/// Grow every auto-height ancestor of a left or right float to the float's
/// bottom edge, including the ancestor's bottom border.
///
/// Taffy does not include floated descendants in an auto-height containing
/// block's used height. This keeps following flow from moving upward after a
/// fragmented flex item with a float descendant.
pub(crate) fn propagate_float_bottoms_to_auto_height_ancestors(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    // Parent map: the arena has no parent pointers, so derive them from children.
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element
            || !doc.nodes[idx].is_in_document()
            || !matches!(
                cascade.computed[idx].float,
                FloatValue::Left | FloatValue::Right
            )
        {
            continue;
        }
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }
}

/// Collapse adjacent block margins inside a non-replaced inline box.
///
/// CSS 2.1 §9.2.1.1 splits an inline containing block around block-level
/// children. Whitespace-only text between those children is part of the
/// anonymous inline fragments, not an intervening block. Taffy's block path
/// already gives the children the right vertical order, but it does not
/// perform this block-in-inline margin collapse; keeping both margins would
/// add one extra line-height-sized gap between two adjacent block children.
/// Keep this pass narrow: only direct children of a plain `inline` wrapper and
/// only resolvable absolute margins are rewritten.
pub(crate) fn collapse_block_in_inline_margins(
    doc: &mut Document,
    idx: usize,
    cascade: &CascadeResult,
) {
    if cascade.computed[idx].display != DisplayValue::Inline {
        return;
    }

    let mut previous_block = None;
    for &child in &doc.nodes[idx].children.clone() {
        if !doc.nodes[child].is_in_document() {
            continue;
        }
        match doc.nodes[child].kind() {
            NodeKind::Text => {
                // Whitespace around a block child belongs to the anonymous
                // inline fragments and must not interrupt sibling margin
                // collapse. Any visible text does interrupt that sequence.
                if !text_of(doc, child).is_some_and(|text| text.chars().all(is_css_white_space)) {
                    previous_block = None;
                }
            }
            NodeKind::Element => {
                let display = cascade.computed[child].display;
                if display == DisplayValue::None {
                    continue;
                }
                if is_inline_element_box(cascade, child) {
                    previous_block = None;
                    continue;
                }
                if let Some(previous) = previous_block {
                    collapse_adjacent_block_margins(doc, previous, child);
                }
                previous_block = Some(child);
            }
            _ => previous_block = None,
        }
    }
}

fn collapse_adjacent_block_margins(doc: &mut Document, previous: usize, next: usize) {
    let previous_margin = doc.nodes[previous].style.margin.bottom.into_raw();
    let next_margin = doc.nodes[next].style.margin.top.into_raw();
    if previous_margin.tag() != CompactLength::LENGTH_TAG
        || next_margin.tag() != CompactLength::LENGTH_TAG
    {
        return;
    }
    let previous_margin = previous_margin.value();
    let next_margin = next_margin.value();
    if !previous_margin.is_finite() || !next_margin.is_finite() {
        return;
    }
    let collapsed = if previous_margin >= 0.0 && next_margin >= 0.0 {
        previous_margin.max(next_margin)
    } else if previous_margin <= 0.0 && next_margin <= 0.0 {
        previous_margin.min(next_margin)
    } else {
        previous_margin + next_margin
    };
    if collapsed.is_finite() {
        doc.nodes[previous].style.margin.bottom = LengthPercentageAuto::length(collapsed);
        doc.nodes[next].style.margin.top = LengthPercentageAuto::length(0.0);
    }
}

/// Return the nearest block-like inline formatting context for a node.
///
/// The nested-inline bridge must not be enabled by an autospace pair in an
/// unrelated sibling block. Low-level DOM tests do not install the HTML UA
/// stylesheet, so if no block-like ancestor is visible the highest reachable
/// ancestor is used as the conservative context fallback.
fn inline_formatting_context_root(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> usize {
    let mut current = node_id;
    let mut fallback = node_id;
    while let Some(parent) = parent_of.get(current).copied().flatten() {
        fallback = parent;
        if matches!(
            cascade.computed[parent].display,
            DisplayValue::Block
                | DisplayValue::InlineBlock
                | DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRow
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return parent;
        }
        current = parent;
    }
    fallback
}

/// Return whether one inline formatting context contains an actual autospace
/// boundary. This reuses the same edge discovery and `text-autospace` option
/// handling as `preshape_text`, instead of combining character classes from
/// unrelated text nodes or ignoring custom values and language gating.
fn inline_context_has_autospace_candidate(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    context_root: usize,
) -> bool {
    for (idx, node) in doc.nodes.iter().enumerate() {
        if node.kind() != NodeKind::Text
            || !node.is_in_document()
            || inline_formatting_context_root(cascade, parent_of, idx) != context_root
            || cascade.computed[idx].text_autospace == TextAutospace::NoAutospace
            || has_vertical_writing_mode(cascade, parent_of, idx)
        {
            continue;
        }
        let NodeData::Text(text) = &node.data else {
            // cov:ignore: NodeKind::Text always carries NodeData::Text; this is a defensive match arm.
            continue;
        };
        let language = effective_language_for_text(doc, parent_of, idx);
        let before = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, -1);
        let after = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, 1);
        if !text_autospace_boxes_with_edges(
            &text.text_content,
            cascade.computed[idx].text_autospace,
            &language,
            1.0,
            before,
            after,
        )
        .is_empty()
        {
            return true;
        }
    }
    false
}

/// Vertical line-box extent contributed by a supported `vertical-align` value.
/// Positive raised offsets extend the line's ascent; negative lowered offsets
/// extend its descent. The paint-side shift is still applied to glyphs.
/// spec: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn vertical_align_linebox_extent(
    vertical_align: VerticalAlign,
    parent_font_size_px: f32,
) -> (f32, f32) {
    let shift = match vertical_align {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        VerticalAlign::Length(Length::Px(px)) => -px,
        _ => 0.0,
    };
    if !shift.is_finite() {
        return (0.0, 0.0);
    }
    if shift < 0.0 {
        (-shift, 0.0)
    } else {
        (0.0, shift)
    }
}

#[derive(Default)]
struct SyntheticLineboxExtents {
    leading: f32,
    trailing: f32,
}

/// Accumulate vertical-align extents from an in-flow inline subtree into one
/// containing synthetic line root. Inline offsets compose along a descendant
/// path. Atomic inline-blocks and nested synthetic roots own their own content
/// line boxes, so stop after accounting for their outer shift.
fn collect_inline_linebox_extents(
    doc: &Document,
    cascade: &CascadeResult,
    synthetic_line_roots: &[bool],
    idx: usize,
    parent_font_size_px: f32,
    ancestor_shift_px: f32,
    extents: &mut SyntheticLineboxExtents,
) {
    if doc.nodes[idx].kind() != NodeKind::Element || !doc.nodes[idx].is_in_document() {
        return;
    }
    let computed = &cascade.computed[idx];
    if computed.display == DisplayValue::None
        || matches!(
            computed.position,
            PositionValue::Absolute | PositionValue::Fixed
        )
        || !matches!(
            computed.display,
            DisplayValue::Inline | DisplayValue::InlineBlock
        )
    {
        return;
    }

    let (leading, trailing) =
        vertical_align_linebox_extent(computed.vertical_align, parent_font_size_px);
    let shifted_baseline = ancestor_shift_px + trailing - leading;
    extents.leading = extents.leading.max(-shifted_baseline);
    extents.trailing = extents.trailing.max(shifted_baseline);

    if computed.display == DisplayValue::InlineBlock
        || synthetic_line_roots.get(idx).copied().unwrap_or(false)
    {
        return;
    }
    let descendant_parent_font_size_px = computed.font_size.px();
    for &child in &doc.nodes[idx].children {
        collect_inline_linebox_extents(
            doc,
            cascade,
            synthetic_line_roots,
            child,
            descendant_parent_font_size_px,
            shifted_baseline,
            extents,
        );
    }
}

/// Add synthetic line-box leading/trailing to authored padding without losing
/// a percentage component. Taffy's calc callback already resolves this mixed
/// value for bridged CSS lengths.
fn padding_with_linebox_extent(
    doc: &mut Document,
    value: ComputedLengthPercentage,
    extra: f32,
    site: &'static str,
) -> LengthPercentage {
    if extra <= 0.0 {
        return computed_length_percentage_to_taffy_length_percentage(
            value,
            site,
            &mut doc.layout_warnings,
        );
    }
    let extra = sanitize_taffy(extra, site, &mut doc.layout_warnings);
    match value {
        ComputedLengthPercentage::Px(px) => {
            LengthPercentage::length(sanitize_taffy(px + extra, site, &mut doc.layout_warnings))
        }
        ComputedLengthPercentage::Percent(percent) => {
            let percent = sanitize_taffy(percent, site, &mut doc.layout_warnings);
            let value = CalcLengthPercentage { percent, px: extra };
            doc.calc_values.push(std::sync::Arc::new(value));
            let pointer = doc
                .calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentage::calc(pointer)
        }
    }
}

pub(crate) fn establish_minimal_line_boxes(doc: &mut Document, cascade: &CascadeResult) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    let mut autospace_candidates_by_root = HashMap::new();
    let mut synthetic_line_roots = vec![false; doc.nodes.len()];
    let mut multicol_roots = vec![false; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE)
        {
            // Laid out by the shodo inline engine, not as a parley line box.
            continue;
        }
        // A multicolumn container establishes fragmentainers rather than the
        // synthetic single flex line used by ordinary inline roots.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        if !matches!(cascade.computed[idx].column_count, ColumnCountValue::Auto)
            // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
            || !matches!(
                cascade.computed[idx].column_width,
                ComputedColumnWidth::Auto
            )
        // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
        {
            multicol_roots[idx] = true;
            continue;
        }
        let has_autospace_candidate = if cascade.computed[idx].display == DisplayValue::Inline {
            let context_root = inline_formatting_context_root(cascade, &parent_of, idx);
            *autospace_candidates_by_root
                .entry(context_root)
                .or_insert_with(|| {
                    inline_context_has_autospace_candidate(doc, cascade, &parent_of, context_root)
                })
        } else {
            false
        };
        synthetic_line_roots[idx] =
            qualifies_for_minimal_line_box(doc, idx, cascade, has_autospace_candidate);
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE)
        {
            // Laid out by the shodo inline engine, not as a parley line box.
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        if multicol_roots[idx] {
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        collapse_block_in_inline_margins(doc, idx, cascade);
        let qualifies = synthetic_line_roots[idx];
        doc.nodes[idx]
            .flags
            .set(NodeFlags::IS_INLINE_ROOT, qualifies);
        if !qualifies {
            continue;
        }
        // This includes display:none children: taffy excludes them entirely
        // from the layout tree with Display::None. Checking their
        // flex_grow/flex_shrink is redundant but harmless.
        let participating_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].is_in_document())
            .collect();
        // Does this container have any `<br>` whose display is not none?
        // This is exactly the condition in this function's "`<br>` forced break"
        // docs. A display:none `<br>` makes no box and must not count:
        // counting it would turn on `flex_wrap: Wrap` for a visually break-free
        // container, unexpectedly enabling size-based wrapping (see docs).
        let has_forced_break = participating_children
            .iter()
            .any(|&c| is_forced_line_break(doc, c, cascade));
        let container_cv = &cascade.computed[idx];
        let has_ch_indent = container_cv.text_indent_ch_factor.is_some()
            && !container_cv.text_indent_hanging
            && !container_cv.text_indent_each_line;
        // The synthetic flex bridge has no full inline formatting context, so
        // text-only roots and plain inline wrappers keep FlexStart. Preserve
        // Taffy's baseline fallback for direct in-flow inline-block and
        // replaced inline-image participants.
        let baseline_aligns_supported_boxes = participating_children.iter().any(|&child| {
            let child_cv = &cascade.computed[child];
            let is_baseline_box = child_cv.display == DisplayValue::InlineBlock
                || (child_cv.display == DisplayValue::Inline
                    && doc.nodes[child].tag_name() == Some("img"));
            is_baseline_box
                && !matches!(
                    child_cv.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
        });
        let mut linebox_extents = SyntheticLineboxExtents::default();
        for &child in &participating_children {
            collect_inline_linebox_extents(
                doc,
                cascade,
                &synthetic_line_roots,
                child,
                container_cv.font_size.px(),
                0.0,
                &mut linebox_extents,
            );
        }
        let linebox_leading = linebox_extents.leading;
        let linebox_trailing = linebox_extents.trailing;
        let linebox_padding_top = (linebox_leading > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.top,
                linebox_leading,
                "line-box-leading",
            )
        });
        let linebox_padding_bottom = (linebox_trailing > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.bottom,
                linebox_trailing,
                "line-box-trailing",
            )
        });
        {
            let style = &mut doc.nodes[idx].style;
            if let Some(padding) = linebox_padding_top {
                style.padding.top = padding;
            }
            if let Some(padding) = linebox_padding_bottom {
                style.padding.bottom = padding;
            }
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = if has_forced_break || has_ch_indent {
                TaffyFlexWrap::Wrap
            } else {
                TaffyFlexWrap::NoWrap
            };
            style.align_items = Some(if baseline_aligns_supported_boxes {
                TaffyAlignItems::BASELINE
            } else {
                TaffyAlignItems::FLEX_START
            });
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            // Center the whole line box for `text-align: center` via flex
            // main-axis alignment, not parley. Keep other values at `None`:
            // flex handling of `Right` and others is future work.
            style.justify_content = if cascade.computed[idx].text_align == TextAlign::Center {
                Some(TaffyAlignContent::CENTER)
            } else {
                None
            };
            style.gap = Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            };
        }
        for c in participating_children {
            let is_break = is_forced_line_break(doc, c, cascade);
            let strip_inline_padding = cascade.computed[c].display == DisplayValue::Inline
                && has_line_box_edge_aligned_descendant(doc, c, cascade);
            let child_style = &mut doc.nodes[c].style;
            if strip_inline_padding {
                // The wrapper is bridged as a block box, so its block-axis
                // padding would move a nested top/bottom-aligned descendant
                // away from the outer line edge. For this narrow edge-aligned
                // slice, remove that padding from the wrapper's block layout.
                child_style.padding.top = LengthPercentage::length(0.0);
                child_style.padding.bottom = LengthPercentage::length(0.0);
            }
            child_style.flex_grow = 0.0;
            child_style.flex_shrink = 0.0;
            child_style.flex_basis = if is_break {
                Dimension::percent(1.0)
            } else {
                Dimension::auto()
            };
            child_style.align_self = match cascade.computed[c].vertical_align {
                VerticalAlign::Top => Some(TaffyAlignSelf::FLEX_START),
                VerticalAlign::Bottom => Some(TaffyAlignSelf::FLEX_END),
                _ => None,
            };
        }
    }
}

/// Whether `idx`, an in-document node, should act as a "`<br>` forced break"
/// in [`establish_minimal_line_boxes`] (see that function's section).
///
/// Check for `NodeKind::Element` with `tag_name() == Some("br")` and computed
/// `display` other than [`DisplayValue::None`]. This direct tag_name comparison
/// follows [`find_body`], which compares `tag_name() == Some("body")`. Identifying
/// `<br>` needs no new [`NodeKind`] variant.
fn is_forced_line_break(doc: &Document, idx: usize, cascade: &CascadeResult) -> bool {
    doc.nodes[idx].kind() == NodeKind::Element
        && doc.nodes[idx].tag_name() == Some("br")
        && cascade.computed[idx].display != DisplayValue::None
}

/// Return whether an inline wrapper contains a descendant whose aligned
/// subtree is explicitly pinned to a line-box edge.
///
/// The minimal bridge may keep a nested inline wrapper on taffy's block path
/// when it has no inline element child. A wrapper with
/// `padding-top`/`padding-bottom` would therefore move both its text and the
/// aligned descendant, unlike the CSS line-box model. The caller uses this
/// predicate only for the focused `top`/`bottom` slice.
fn has_line_box_edge_aligned_descendant(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
) -> bool {
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(child) = stack.pop() {
        if doc.nodes[child].kind() == NodeKind::Element {
            if matches!(
                cascade.computed[child].vertical_align,
                VerticalAlign::Top | VerticalAlign::Bottom
            ) {
                return true;
            }
            stack.extend(doc.nodes[child].children.iter().copied());
        }
    }
    false
}

/// Whether `idx` (a [`NodeKind::Element`]) qualifies for the minimal line-box
/// handling in [`establish_minimal_line_boxes`]. See that function's docs
/// for the qualifying condition and rationale.
fn qualifies_for_minimal_line_box(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
    has_autospace_candidate: bool,
) -> bool {
    // Deliberately read the pre-bridge `DisplayValue`, not the post-
    // `bridge_display` `taffy::Display`: by this second pass, `Block`,
    // `Inline`, and `InlineBlock` have all collapsed to `Display::Block`.
    // Exclude plain `Inline` with only direct text children; qualify only
    // wrappers with a nested inline element child (see module docs).
    // Qualify table types (`Table` / `InlineTable` / `TableRow` /
    // `TableCell` / `TableCaption`) only with all-inline children; see
    // the match-arm note. The table dispatch in `taffy_impl.rs` checks
    // `IS_INLINE_ROOT` to bypass the table engine.
    let is_plain_inline = has_autospace_candidate
        && cascade.computed[idx].display == DisplayValue::Inline
        && cascade.computed[idx].text_autospace != TextAutospace::NoAutospace;
    if cascade.computed[idx].display == DisplayValue::Inline && !is_plain_inline {
        return false;
    }
    let has_inline_element_child = is_plain_inline
        && doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element && is_inline_element_box(cascade, child)
        });
    match cascade.computed[idx].display {
        DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::Inline => {}
        // Table-internal boxes whose children are ALL inline-level get no
        // anonymous fixup from the table engine (it only collects rows and
        // cells) — without this they stack vertically in taffy's block
        // path. A pure-inline table/row/cell/caption is exactly one
        // anonymous cell's content (css-tables-3 §2.2.1 fixup with a single
        // run), so flowing it inline here is equivalent. Boxes with any
        // cell/row (or other block-level) child keep the table path via the
        // disqualify arm below. Column groups/columns never have renderable
        // children and stay disqualified.
        DisplayValue::Table
        | DisplayValue::InlineTable
        | DisplayValue::TableRow
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => {}
        _ => return false,
    }
    let mut inline_level_count = 0usize;
    for &c in &doc.nodes[idx].children {
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => inline_level_count += 1,
            NodeKind::Element => match cascade.computed[c].display {
                DisplayValue::Inline | DisplayValue::InlineBlock => inline_level_count += 1,
                DisplayValue::None => {}
                // Block-level / flex / grid children, as well as future
                // non_exhaustive DisplayValue variants, disqualify the parent:
                // mixed block-level and inline-level content is out of scope
                // (see module docs).
                _ => return false,
            },
            // Comment / ProcessingInstruction / DocumentFragment /
            // Document should never satisfy `is_in_document()` (see
            // `NodeData` docs); fail closed here without assuming that invariant.
            // cov:ignore: unreachable — Comment/ProcessingInstruction/DocumentFragment/Document nodes always have IS_IN_DOCUMENT cleared (see document.rs's mark_in_document_flags invariant), filtered by the is_in_document() guard above; kept as a defensive fallback rather than relying on that invariant here.
            _ => return false,
        }
    }
    if is_plain_inline {
        // A plain inline with a nested inline element needs a synthetic line
        // container so nested descendants do not take the block path and
        // stack vertically. Keep direct text-only inline containers on the
        // historical path; their content already participates in the parent
        // line box and existing layout behavior remains unchanged.
        has_inline_element_child && inline_level_count >= 1
    } else {
        inline_level_count >= 2
    }
}

/// Minimum valid `font-weight` passed to parley.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop> specifies
/// `<number [1,1000]>`. This sink (`parley::FontWeight`) is distinct from
/// [`MAX_TAFFY_MAGNITUDE`] / [`MAX_FONT_SIZE_PX`], so use the spec range
/// rather than a shared bound: limits depend on the sink. Unlike skrifa /
/// CSSWG issues, this site needs no engineering measurement.
const MIN_FONT_WEIGHT: f32 = 1.0;

/// Maximum valid `font-weight` passed to parley (CSS Fonts 4 §2.2).
const MAX_FONT_WEIGHT: f32 = 1000.0;

/// Fallback for non-finite (`NaN`) `font-weight`.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> assigns
/// `normal` this computed value, matching `ComputedValues::initial().
/// font_weight` (see `crates/raikiri-style/src/computed.rs`).
///
/// [`sanitize_finite`] maps NaN to `0.0` at length sinks because the spec's
/// initial padding/margin is geometric zero or because infinite-precision
/// `0 * inf = NaN` should evaluate to zero. Neither justification applies
/// to font-weight. `0.0` is outside `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]`;
/// using it for NaN would leave the sanitized value outside the sink's valid
/// range. Font-weight therefore has its own fallback.
const FALLBACK_FONT_WEIGHT: f32 = 400.0;

/// Sanitize non-finite (`NaN` / `±Inf`) or out-of-range
/// `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]` `font-weight` immediately before
/// passing it to `parley::FontWeight::new`.
///
/// # Why guard here?
///
/// Guard non-finite / out-of-range f32 values at the sink where they are used
/// (target context), not at parse time (specified layer) or during resolution
/// (computed layer). The inherited root argument of
/// [`raikiri_style::page::cascade_page`] (raikiri-style), direct construction
/// of `ComputedValues`, and `raikiri_style::cascade::resolve_relative_weight`
/// all belong to the resolve/computed layer. This policy expressly forbids
/// adding guards there: "do not sanitize public computed-layer surfaces".
/// Placing this function at the parley text probes, the sole callers of
/// `parley::FontWeight::new`, matches the sink-adjacent guards at the other
/// sites (the taffy bridge helpers).
///
/// # Why use [`FALLBACK_FONT_WEIGHT`] (`400.0`) rather than `0.0` for NaN?
///
/// Do not reuse [`sanitize_finite`] directly, for two reasons:
///
/// 1. **Different NaN fallback.** [`sanitize_finite`] unconditionally maps
///    NaN to `0.0`, but its rationale (see its docs) does not apply to
///    font-weight. Zero is outside the valid range. CSS Fonts 4 §2.2
///    "Font weight: the font-weight property"
///    <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> gives
///    `normal` the computed value `400.0`; using it follows the policy that
///    fallbacks and limits vary by sink.
/// 2. **Cost of changing the signature.** Adding a `nan_fallback` parameter
///    could unify the functions in theory, but it would add an irrelevant
///    argument to the existing length-related call sites (four via
///    `sanitize_taffy` plus one direct font-size call) and all their tests.
///    A small separate function keeps the diff proportional to the single
///    font-weight sink in scope.
///
/// # Do not change the producer (`resolve_relative_weight`)
///
/// Do not alter `resolve_relative_weight`'s asymmetric handling of `Bolder`,
/// `Lighter`, and `-Inf` (see its docs). Changing resolution is forbidden by
/// this policy; this sink guard only ensures that whatever the resolver
/// produces ends up finite and within the valid range.
fn sanitize_font_weight(v: f32, diag: &mut Vec<LayoutWarn>) -> f32 {
    let clamped = if v.is_nan() {
        FALLBACK_FONT_WEIGHT
    } else {
        v.clamp(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT)
    };
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site: "font-weight",
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// `raikiri_style::property::FontStyle` (`Normal | Italic | Oblique`,
/// `#[non_exhaustive]`) → parley's `FontStyle` (re-exported from the
/// `parlance` crate: `Normal | Italic | Oblique(Option<f32>)`, CSS Fonts 4
/// §2.4 <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// `Oblique` has no `<angle>` payload on the raikiri-style side (bare
/// keyword only — see `raikiri_style::property::FontStyle` doc's "Scope
/// carving" section), so it maps to parley's `Oblique(None)`, which per
/// parley's own doc uses the engine-specific default oblique angle. The
/// wildcard arm exists purely for `StyleFontStyle`'s `#[non_exhaustive]`
/// forward-compat contract (a downstream match must tolerate variants
/// added to the source enum later, e.g. a future `<angle>`-bearing
/// oblique) and is unreachable with the variant set that exists today.
fn font_style_to_parley(v: StyleFontStyle) -> FontStyle {
    match v {
        StyleFontStyle::Normal => FontStyle::Normal,
        StyleFontStyle::Italic => FontStyle::Italic,
        StyleFontStyle::Oblique => FontStyle::Oblique(None),
        // cov:ignore: unreachable while StyleFontStyle is
        // Normal|Italic|Oblique only; required for its #[non_exhaustive]
        // contract (see doc above).
        _ => FontStyle::Normal,
    }
}

fn probe_ch_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        "0",
        family_str,
        font_size_px,
        font_weight,
        font_style,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn probe_text_advance_inner(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    allow_zero: bool,
) -> f32 {
    let mut warnings: Vec<LayoutWarn> = Vec::new();
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(font_weight, &mut warnings);
    let family = FontFamily::from(family_str);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(family));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    let w = layout.width();
    let has_glyph_run = allow_zero
        && layout
            .lines()
            .flat_map(|line| line.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
                _ => false, // cov:ignore: this helper shapes plain text and cannot create non-glyph items.
            });
    if w.is_finite() && (w > 0.0 || has_glyph_run) && w <= size * 4.0 {
        w
    } else {
        size * 0.5
    }
}

/// Return whether the requested face maps both the space and zero glyphs.
///
/// CSS's first-available `ch` face must be usable for the space and must also
/// provide U+0030, whose advance supplies the metric. A mapped zero-advance
/// glyph is valid; cmap presence is the only coverage check here.
fn family_candidate_has_ch_glyphs(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Return whether any face in a generic family maps both required glyphs.
fn generic_family_has_ch_glyphs(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Measure CSS `ch` from the selected font's U+0030 advance.
///
/// Uses the same font-selection and glyph-coverage checks as the layout sink.
/// If no selected face provides a usable zero glyph, this returns the
/// deterministic `0.5em` fallback used by style resolution.
///
/// [`raikiri_style::ChFontKey`] preserves the declaring element's family,
/// size, weight, and style so callers can resolve authored `ch` provenance
/// after font-face registration.
pub fn measure_ch_advance_for_font_key(
    fonts: &mut FontContext,
    key: &raikiri_style::ChFontKey,
) -> f32 {
    let family = key
        .family
        .iter()
        .map(|family| family.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut layout_cx = LayoutContext::<()>::new();
    probe_ch_text_advance(
        fonts,
        &mut layout_cx,
        &family,
        key.size.px(),
        key.weight,
        key.style,
    )
}

/// Probe U+0030 using the first remaining family that can supply the glyph.
///
/// Named faces are checked through Fontique cmap metadata. An unregistered
/// name is skipped so a later available family can win; when a generic family
/// is reached, Parley receives the remaining authored list so its fallback
/// resolver preserves generic and later-family order. A valid zero-advance
/// mapped glyph remains accepted by `probe_ch_zero_advance`.
fn probe_ch_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        if raw_candidate.is_empty() {
            // cov:ignore: candidates were already filtered for emptiness.
            continue;
        }
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            None
        } else {
            let lower_name = name.to_ascii_lowercase();
            parley::fontique::GenericFamily::parse(&lower_name)
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        let has_glyphs =
            registered && family_candidate_has_ch_glyphs(fonts, name, font_weight, font_style);
        if let Some(generic) = generic {
            // An unresolved named face may represent an @font-face source
            // that the WPT loader could not activate (for example a missing
            // or malformed web-font resource). Do not silently replace that unavailable
            // face with a generic metric; the style-layer fallback is the
            // deterministic 0.5em result for this no-face case.
            if saw_unregistered_named
                || !generic_family_has_ch_glyphs(fonts, generic, font_weight, font_style)
            {
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if has_glyphs {
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    // No remaining family advertises U+0030. Do not shape the rejected list
    // again: Parley may emit a .notdef run whose advance would masquerade as
    // a valid zero-width glyph. Match the style-layer fallback instead.
    let mut warnings = Vec::new();
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    ) * 0.5
}

pub(crate) fn bounded_text_indent_amount(
    value: ComputedTextIndent,
    containing_width: f32,
    measured: Option<f32>,
) -> f32 {
    let raw = match value {
        ComputedTextIndent::Px(px) => measured.unwrap_or(px),
        ComputedTextIndent::Percent(percent) => containing_width * percent / 100.0,
        // A measured `ch` calc already folds its absolute part into `measured`,
        // so only the percentage term is added on top.
        ComputedTextIndent::Calc(calc) => match measured {
            Some(measured) => measured + containing_width * calc.percent / 100.0,
            None => calc.px + containing_width * calc.percent / 100.0,
        },
    };
    if raw.is_nan() {
        0.0
    } else {
        raw.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

/// [`ComputedLength`]: raikiri_style::ComputedLength
/// [`ComputedLineHeight`]: raikiri_style::ComputedLineHeight
/// CSS white-space characters (CSS Text 3 §4.1.1): space, tab, LF, FF, CR.
fn is_css_white_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Zero-width space (U+200B): a segment break adjacent to it is removed
/// without leaving a space (CSS Text 3 §4.1.2 rule 1).
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// East Asian Width map for segment-break decisions (CSS Text 3 §4.1.2
/// rule 2: removal when both sides are Fullwidth/Wide/Halfwidth).
/// Loaded once per process from ICU compiled data (same provider the
/// parley dependency already links; no new data pulled in).
fn east_asian_width_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::EastAsianWidth> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::EastAsianWidth>::new()
}

/// Whether `c` is a default-ignorable code point for segment-break
/// neighbor selection, excluding U+200B whose explicit rule is handled by
/// the caller.
fn is_segment_break_ignorable(c: char) -> bool {
    // U+200B has an explicit segment-break rule and must remain visible to
    // the caller; other default-ignorable code points (variation selectors,
    // soft hyphen, LRM, etc.) do not participate in the EAW neighbor test.
    c != ZERO_WIDTH_SPACE
        && icu_properties::CodePointSetData::new::<
            icu_properties::props::DefaultIgnorableCodePoint,
        >()
        .contains(c)
}

/// Whether `idx` generates an inline-level box for white-space trimming.
///
/// Text nodes are inline-level; elements follow their computed display
/// (`inline`, `inline-block`, `inline-table`). `contents` is treated as
/// inline (conservative: a wrong keep preserves old behavior, a wrong drop
/// would regress). `<br>` counts as a block boundary (leading/trailing
/// spaces around a forced break collapse).
fn is_inline_for_trim(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    if doc.nodes[idx].kind() == NodeKind::Text {
        return true;
    }
    // Absolutely/fixed-positioned boxes are out of the inline flow and cannot
    // keep a collapsible space at the boundary of the following text.
    if matches!(
        cascade.computed[idx].position,
        PositionValue::Absolute | PositionValue::Fixed
    ) {
        return false;
    }
    if doc.nodes[idx].tag_name() == Some("br") {
        return false;
    }
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Whether the element's own box is inline-level (strict: `contents` and
/// `<br>` excluded — used for ancestor ascent, where a `contents` wrapper
/// must not stop the search but a real inline box does not continue it...
/// actually ascent continues through ALL inline ancestors; this helper
/// reports whether `idx` (an element) generates an inline-level box,
/// `<br>` included as inline for ascent since content inside... `<br>` has
/// no children, so it never matters here).
pub(crate) fn is_inline_element_box(cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Find the nearest in-flow inline character on one side of a text node.
///
/// Text nodes are shaped independently, but autospace applies across ordinary
/// inline-element boundaries. Whitespace, block-level boxes, atomic inline
/// boxes, and isolation boundaries stop the search rather than being skipped;
/// default-ignorable code points such as variation selectors are looked past.
pub(crate) fn autospace_adjacent_edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let parent = parent_of[node]?;
        let children = &doc.nodes[parent].children;
        let position = children.iter().position(|&child| child == node)?;
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(children[..position].iter().rev())
        } else {
            Box::new(children[position + 1..].iter())
        };
        for &sibling in siblings {
            match boundary_inline_edge(doc, cascade, sibling, dir, is_segment_break_ignorable) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return None,
                InlineEdge::Char(character) => return Some(character),
            }
        }
        if doc.nodes[parent].kind() == NodeKind::Element
            && is_inline_element_box(cascade, parent)
            && !is_shaping_isolation_boundary(doc, parent)
            && !boundary_shaping_box_breaks(cascade, parent)
        {
            node = parent;
            continue;
        }
        return None;
    }
}

/// Whether an element creates a bidi isolation boundary for shaping.
///
/// The current style bridge does not expose `unicode-bidi` in computed values,
/// so cover the HTML isolation forms used by the WPT boundary-shaping tests:
/// `<bdi>` and an inline element with `dir="auto"`.
fn is_shaping_isolation_boundary(doc: &Document, node_id: usize) -> bool {
    doc.nodes[node_id].tag_name() == Some("bdi")
        || doc.nodes[node_id]
            .attribute("dir")
            .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
}

/// Whether a boundary box prevents joining across its inline content.
///
/// Nonzero physical inline-edge margins, padding, and borders are shaping
/// boundaries; outline and text decoration are not. The current style bridge
/// normalizes writing mode to horizontal-tb, so the inline edges are left and
/// right. Atomic inline-level boxes also cannot share a shaping context with
/// their siblings.
fn boundary_shaping_box_breaks(cascade: &CascadeResult, node_id: usize) -> bool {
    let cv = &cascade.computed[node_id];
    if !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
        return true;
    }
    let nonzero_length_percentage = |value: ComputedLengthPercentage| match value {
        ComputedLengthPercentage::Px(px) | ComputedLengthPercentage::Percent(px) => {
            px.abs() > f32::EPSILON
        }
    };
    let nonzero_length_percentage_or_auto = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() > f32::EPSILON
        }
        ComputedLengthPercentageOrAuto::Calc(_) => true,
        ComputedLengthPercentageOrAuto::Auto => false,
    };
    [cv.margin.left, cv.margin.right]
        .into_iter()
        .any(nonzero_length_percentage_or_auto)
        || [cv.padding.left, cv.padding.right]
            .into_iter()
            .any(nonzero_length_percentage)
        || [cv.border.left.width().px(), cv.border.right.width().px()]
            .into_iter()
            .any(|width| width.abs() > f32::EPSILON)
}

/// What a subtree contributes at the edge facing a shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineEdge {
    /// The subtree renders nothing inline; look further along the boundary.
    Empty,
    /// The subtree ends the joining context before any character.
    Break,
    /// The first rendered character met from the boundary side.
    Char(char),
}

/// Walk into `idx` from the side facing the boundary (`dir` +1 enters from
/// the start, -1 from the end) and report the first character it renders,
/// ignoring characters for which `skip` returns true. Out-of-flow and `display:none` boxes render nothing inline; atomic
/// inlines, `<br>`, bidi isolates, and inline boxes with nonzero inline-edge
/// margin/border/padding break the context.
fn boundary_inline_edge(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    dir: i8,
    skip: fn(char) -> bool,
) -> InlineEdge {
    let node = &doc.nodes[idx];
    if !node.is_in_document() {
        return InlineEdge::Empty;
    }
    if let Some(text) = text_of(doc, idx) {
        let edge = if dir < 0 {
            text.chars().rev().find(|&ch| !skip(ch))
        } else {
            text.chars().find(|&ch| !skip(ch))
        };
        return edge.map_or(InlineEdge::Empty, InlineEdge::Char);
    }
    // Comments and processing instructions never carry the in-document flag,
    // so any remaining node here is an element.
    let cv = &cascade.computed[idx];
    if cv.display == DisplayValue::None
        || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return InlineEdge::Empty;
    }
    if !is_inline_for_trim(doc, cascade, idx)
        || is_shaping_isolation_boundary(doc, idx)
        || boundary_shaping_box_breaks(cascade, idx)
    {
        return InlineEdge::Break;
    }
    let children = &node.children;
    let ordered: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
        Box::new(children.iter().rev())
    } else {
        Box::new(children.iter())
    };
    for &child in ordered {
        match boundary_inline_edge(doc, cascade, child, dir, skip) {
            InlineEdge::Empty => continue,
            edge => return edge,
        }
    }
    InlineEdge::Empty
}

/// Raw text content of a text node.
fn text_of(doc: &Document, idx: usize) -> Option<&str> {
    match &doc.nodes[idx].data {
        crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
        _ => None,
    }
}

/// A coarse CSS Text 4 autospace class.  The style layer preserves the
/// complete `text-autospace` value; this layout slice only needs the boundary
/// classes to create non-painting in-flow advances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextAutospaceClass {
    Ideograph,
    Letter,
    Numeric,
    Other,
}

fn text_autospace_class(c: char) -> TextAutospaceClass {
    use icu_properties::props::{GeneralCategoryGroup, Script};

    let gc =
        icu_properties::CodePointMapDataBorrowed::<icu_properties::props::GeneralCategory>::new()
            .get(c);
    let eaw = east_asian_width_map().get(c);
    let wide = matches!(
        eaw,
        icu_properties::props::EastAsianWidth::Fullwidth
            | icu_properties::props::EastAsianWidth::Wide
    );
    // CSS Text's ideographic set includes Han and the Japanese kana/stroke
    // ranges. Punctuation in the shared ranges is not an ideograph.
    let ideograph =
        (icu_properties::CodePointMapDataBorrowed::<icu_properties::props::Script>::new().get(c)
            == Script::Han
            || matches!(c, '\u{3041}'..='\u{30ff}' | '\u{31c0}'..='\u{31ff}'))
            && !GeneralCategoryGroup::Punctuation.contains(gc);
    if ideograph {
        return TextAutospaceClass::Ideograph;
    }
    if !wide
        && (GeneralCategoryGroup::Letter.contains(gc) || GeneralCategoryGroup::Mark.contains(gc))
    {
        return TextAutospaceClass::Letter;
    }
    if !wide && GeneralCategoryGroup::Number.contains(gc) {
        return TextAutospaceClass::Numeric;
    }
    TextAutospaceClass::Other
}

fn text_autospace_ascii_punctuation(c: char) -> bool {
    // CSS Text 4's `punctuation` class is language-sensitive. Chromium's
    // current behavior (and the zh WPT slice) inserts around these ASCII
    // punctuation marks, but not around fullwidth/CJK punctuation.
    matches!(c, '!' | '#' | ':' | ';' | '?')
}

#[cfg(test)]
pub(crate) fn text_autospace_boxes(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_edges(text, value, language, font_size, None, None)
}

fn text_autospace_boxes_with_edges(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_width(
        text,
        value,
        language,
        (font_size * 0.125).max(0.0),
        before,
        after,
    )
}

fn text_autospace_boxes_with_width(
    text: &str,
    value: TextAutospace,
    language: &str,
    width: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    let (ideograph_alpha, ideograph_numeric, punctuation) = match value {
        TextAutospace::Normal | TextAutospace::Auto => {
            (true, true, language_matches(language, "zh"))
        }
        TextAutospace::NoAutospace => (false, false, false),
        TextAutospace::Custom {
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
            ..
        } => (
            ideograph_alpha,
            ideograph_numeric,
            punctuation && language_matches(language, "zh"),
        ),
        // `TextAutospace` is non-exhaustive so downstream crates remain
        // source-compatible when the style layer gains another keyword.
        _ => (true, true, language_matches(language, "zh")), // cov:ignore: no future non-exhaustive variant exists in the pinned style crate.
    };
    if !ideograph_alpha && !ideograph_numeric && !punctuation {
        return Vec::new();
    }
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }

    let mut previous =
        before.and_then(|c| (!is_segment_break_ignorable(c)).then(|| (c, text_autospace_class(c))));
    let mut last = None;
    let mut boxes = Vec::new();
    for (index, c) in text.char_indices() {
        // Default-ignorable characters, including variation selectors, do not
        // break the neighboring-character test. The VS WPT expects `国` + VS
        // + `A` to use the same autospace boundary as `国A`.
        if is_segment_break_ignorable(c) {
            continue;
        }
        let class = text_autospace_class(c);
        if let Some((previous_char, previous_class)) = previous
            && text_autospace_pair_needs_box(
                previous_char,
                previous_class,
                c,
                class,
                ideograph_alpha,
                ideograph_numeric,
                punctuation,
            )
        {
            boxes.push(InlineBox {
                // Reserve the high ID range for autospace boxes so paint can
                // distinguish their inline baseline semantics from tabs.
                id: u64::MAX - boxes.len() as u64,
                kind: InlineBoxKind::InFlow,
                index,
                width,
                height: 0.0,
            });
        }
        previous = Some((c, class));
        last = Some((c, class));
    }
    if let (Some((previous_char, previous_class)), Some(next)) = (last, after)
        && !is_segment_break_ignorable(next)
        && text_autospace_pair_needs_box(
            previous_char,
            previous_class,
            next,
            text_autospace_class(next),
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
        )
    {
        boxes.push(InlineBox {
            // Reserve the high ID range for autospace boxes so paint can
            // distinguish their inline baseline semantics from tabs.
            id: u64::MAX - boxes.len() as u64,
            kind: InlineBoxKind::InFlow,
            index: text.len(),
            width,
            height: 0.0,
        });
    }
    boxes
}

fn text_autospace_pair_needs_box(
    previous_char: char,
    previous_class: TextAutospaceClass,
    current_char: char,
    current_class: TextAutospaceClass,
    ideograph_alpha: bool,
    ideograph_numeric: bool,
    punctuation: bool,
) -> bool {
    let class_boundary = match (previous_class, current_class) {
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Letter)
        | (TextAutospaceClass::Letter, TextAutospaceClass::Ideograph) => ideograph_alpha,
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Numeric)
        | (TextAutospaceClass::Numeric, TextAutospaceClass::Ideograph) => ideograph_numeric,
        _ => false,
    };
    let punctuation_boundary = punctuation
        && ((text_autospace_ascii_punctuation(previous_char)
            && current_class == TextAutospaceClass::Ideograph)
            || (previous_class == TextAutospaceClass::Ideograph
                && text_autospace_ascii_punctuation(current_char)));
    class_boundary || punctuation_boundary
}

fn effective_language_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_idx: usize,
) -> String {
    let mut current = parent_of[text_idx];
    while let Some(idx) = current {
        if let crate::node::NodeData::Element(element) = &doc.nodes[idx].data
            && let Some(attr) = element.attributes.iter().find(|attr| {
                (attr.namespace.is_none() && attr.local.eq_ignore_ascii_case("lang"))
                    || (attr.namespace.as_deref() == Some("http://www.w3.org/XML/1998/namespace")
                        && attr.local == "lang")
            })
        {
            return attr.value.trim().to_ascii_lowercase();
        }
        current = parent_of[idx];
    }
    String::new()
}

fn language_matches(language: &str, primary: &str) -> bool {
    language == primary
        || language
            .strip_prefix(primary)
            .is_some_and(|rest| rest.starts_with('-'))
}

#[cfg(test)]
mod tests;
